//! Office documents through LibreOffice (`soffice --headless --convert-to`). The input is copied
//! into a private temp folder so LibreOffice never writes lock files next to the user's file,
//! and it runs with Tessera's own hardened profile (macros, remote links, OLE and DDE off).
//! Two processes on one profile make the second fail, so each of the few parallel slots has
//! its own profile (`office-profile`, `office-profile-1`, ...).

use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{allows, output, security, uses_office, Conflict, ConvertError, Format, Options};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// A document that takes longer than this is treated as stuck.
const TIMEOUT: Duration = Duration::from_secs(300);

/// Most LibreOffice processes at once; each takes a few hundred MB of memory.
const MAX_SLOTS: usize = 3;
static SLOTS: [Mutex<()>; MAX_SLOTS] = [const { Mutex::new(()) }; MAX_SLOTS];

/// Slots in use on this PC: half the cores, 1 to `MAX_SLOTS`.
fn slot_count() -> usize {
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    (cores / 2).clamp(1, MAX_SLOTS)
}

/// Profile of slot `i`: the base profile for slot 0, `<base>-<i>` for the others.
fn slot_profile(base: &Path, i: usize) -> PathBuf {
    if i == 0 {
        return base.to_path_buf();
    }
    let mut name = base.as_os_str().to_owned();
    name.push(format!("-{i}"));
    PathBuf::from(name)
}

/// Waits for a free slot (or the cancel flag) and returns its lock and profile.
fn take_slot<'a>(
    base: &Path,
    opts: &Options,
) -> Result<(std::sync::MutexGuard<'a, ()>, PathBuf), ConvertError> {
    loop {
        for (i, slot) in SLOTS.iter().take(slot_count()).enumerate() {
            match slot.try_lock() {
                Ok(g) => return Ok((g, slot_profile(base, i))),
                Err(std::sync::TryLockError::Poisoned(e)) => {
                    return Ok((e.into_inner(), slot_profile(base, i)))
                }
                Err(std::sync::TryLockError::WouldBlock) => {}
            }
        }
        opts.check_cancel()?;
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Written into a fresh profile before LibreOffice first starts (see development/OFFICE.md).
const HARDENING: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<oor:items xmlns:oor="http://openoffice.org/2001/registry" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="DisableMacrosExecution" oor:op="fuse"><value>true</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="MacroSecurityLevel" oor:op="fuse"><value>3</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="BlockUntrustedRefererLinks" oor:op="fuse"><value>true</value></prop></item>
<item oor:path="/org.openoffice.Office.Common/Security/Scripting"><prop oor:name="DisableActiveContent" oor:op="fuse"><value>true</value></prop></item>
<item oor:path="/org.openoffice.Office.Writer/Content/Update"><prop oor:name="Link" oor:op="fuse"><value>0</value></prop></item>
<item oor:path="/org.openoffice.Office.Calc/Content/Update"><prop oor:name="Link" oor:op="fuse"><value>1</value></prop></item>
</oor:items>
"#;

/// `--convert-to` argument: extension plus the export filter that writes it.
fn export_filter(target: Format) -> &'static str {
    match target {
        Format::Docx => "docx:MS Word 2007 XML",
        Format::Doc => "doc:MS Word 97",
        Format::Odt => "odt:writer8",
        Format::Rtf => "rtf:Rich Text Format",
        Format::Txt => "txt:Text (encoded):UTF8",
        Format::Xlsx => "xlsx:Calc MS Excel 2007 XML",
        Format::Xls => "xls:MS Excel 97",
        Format::Ods => "ods:calc8",
        // comma, double quotes only where needed, UTF-8, cells as shown
        Format::Csv => "csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,true",
        Format::Pptx => "pptx:Impress MS PowerPoint 2007 XML",
        Format::Ppt => "ppt:MS PowerPoint 97",
        Format::Odp => "odp:impress8",
        // LibreOffice picks the Writer, Calc or Impress PDF export from the document
        _ => "pdf",
    }
}

/// `--infilter` for sources LibreOffice would guess wrong or not at all.
fn import_filter(from: Format) -> Option<&'static str> {
    match from {
        Format::Pdf => Some("writer_pdf_import"),
        Format::Txt => Some("Text (encoded):UTF8"),
        // comma, semicolon or tab, double quotes, UTF-8
        Format::Csv => Some("Text - txt - csv (StarCalc):44/59/9,34,76"),
        _ => None,
    }
}

/// Rejects files whose bytes do not match their extension before LibreOffice sees them:
/// it would load garbage as plain text and "convert" it. An OOXML or ODF name on an OLE
/// container is an encrypted (password protected) document.
fn check_magic(path: &Path, from: Format) -> Result<(), ConvertError> {
    let mut head = [0u8; 8];
    let n = {
        use std::io::Read;
        fs::File::open(path)?.read(&mut head)?
    };
    let head = &head[..n];
    let zip = head.starts_with(b"PK\x03\x04");
    let ole = head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]);
    match from {
        Format::Docx | Format::Xlsx | Format::Pptx | Format::Odt | Format::Ods | Format::Odp => {
            if zip {
                Ok(())
            } else if ole {
                Err(ConvertError::Protected)
            } else {
                Err(ConvertError::Corrupt)
            }
        }
        Format::Rtf if !head.starts_with(b"{\\rtf") => Err(ConvertError::Corrupt),
        Format::Pdf if !head.starts_with(b"%PDF") => Err(ConvertError::Corrupt),
        _ => Ok(()),
    }
}

/// `file:///C:/a%20b/c` for LibreOffice's `-env:UserInstallation`.
fn file_url(path: &Path) -> String {
    let mut url = String::from("file:///");
    for b in path.to_string_lossy().replace('\\', "/").bytes() {
        if b.is_ascii_alphanumeric() || b"/:-._~".contains(&b) {
            url.push(b as char);
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

/// Seeds the profile with the hardening settings the first time it is used.
fn ensure_profile(profile: &Path) -> Result<(), ConvertError> {
    let xcu = profile.join("user").join("registrymodifications.xcu");
    if !xcu.exists() {
        fs::create_dir_all(xcu.parent().unwrap_or(profile))?;
        fs::write(&xcu, HARDENING)?;
    }
    Ok(())
}

/// Ends soffice.exe and the soffice.bin it started.
fn kill_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();
}

/// A private folder for one run, removed when dropped.
pub(crate) struct Scratch(pub(crate) PathBuf);

impl Scratch {
    pub(crate) fn new() -> std::io::Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("tessera-office-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir)?;
        Ok(Scratch(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs soffice on `input` (already in `dir`) and returns the written file.
fn run_soffice(
    soffice: &Path,
    profile: &Path,
    input: &Path,
    from: Format,
    target: Format,
    dir: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<PathBuf, ConvertError> {
    let mut cmd = Command::new(soffice);
    cmd.current_dir(dir)
        .arg(format!("-env:UserInstallation={}", file_url(profile)))
        .args([
            "--headless",
            "--norestore",
            "--nologo",
            "--nodefault",
            "--nolockcheck",
        ]);
    if let Some(f) = import_filter(from) {
        cmd.arg(format!("--infilter={f}"));
    }
    let mut child = cmd
        .arg("--convert-to")
        .arg(export_filter(target))
        .arg("--outdir")
        .arg(dir)
        .arg(input)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| ConvertError::Encode(format!("LibreOffice: {e}")))?;
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            kill_tree(child.id());
            let _ = child.wait();
            return Err(ConvertError::Cancelled);
        }
        if started.elapsed() > TIMEOUT {
            kill_tree(child.id());
            let _ = child.wait();
            return Err(ConvertError::Encode(
                "LibreOffice did not finish in 5 minutes".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // soffice exits 0 even when it could not load the file: only the output tells
    let out = input.with_extension(target.ext());
    match fs::metadata(&out) {
        Ok(m) if m.len() > 0 => Ok(out),
        _ => Err(ConvertError::Corrupt),
    }
}

/// Runs `f` while no conversion is running (swapping the LibreOffice folder on an update).
pub fn exclusive<T>(f: impl FnOnce() -> T) -> T {
    let _all: Vec<_> = SLOTS
        .iter()
        .map(|s| s.lock().unwrap_or_else(|e| e.into_inner()))
        .collect();
    f()
}

/// Every profile folder the slots may have made from `base` (for removal).
pub fn slot_profiles(base: &Path) -> Vec<PathBuf> {
    (0..MAX_SLOTS).map(|i| slot_profile(base, i)).collect()
}

/// Converts `input` with LibreOffice at `soffice`, using `profile` as its user profile.
/// `progress` gets 0.0 when the job starts and 1.0 when it is written.
pub fn convert_with_office(
    soffice: &Path,
    profile: &Path,
    input: &Path,
    target: Format,
    opts: &Options,
    mut progress: impl FnMut(f32),
) -> Result<PathBuf, ConvertError> {
    let opts = &opts.clamped();
    let (src, from) = security::validate_input(input)?;
    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !allows(ext, from, target, opts) || !uses_office(from, target) {
        return Err(ConvertError::Unsupported);
    }
    check_magic(&src, from)?;
    let dest = output::plan(&src, target.ext(), &opts.output)?;
    progress(0.0);

    let (_turn, profile) = take_slot(profile, opts)?;
    let profile = profile.as_path();
    opts.check_cancel()?;
    ensure_profile(profile)?;
    let scratch = Scratch::new()?;
    // a plain ASCII name: LibreOffice names its output after the input
    let local = scratch.0.join(format!("in.{}", from.ext()));
    fs::copy(&src, &local)?;
    let out = run_soffice(
        soffice,
        profile,
        &local,
        from,
        target,
        &scratch.0,
        opts.cancel.as_deref(),
    )?;
    output::write_atomic(&dest, |w| {
        std::io::copy(&mut fs::File::open(&out)?, w)?;
        Ok(())
    })?;
    if opts.output.conflict == Conflict::ReplaceOriginal {
        output::recycle(&src)?;
    }
    progress(1.0);
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_get_their_own_profiles() {
        let base = Path::new(r"C:\d\office-profile");
        assert_eq!(slot_profile(base, 0), base);
        assert_eq!(slot_profile(base, 2), Path::new(r"C:\d\office-profile-2"));
        assert_eq!(slot_profiles(base).len(), MAX_SLOTS);
        assert!((1..=MAX_SLOTS).contains(&slot_count()));
    }

    #[test]
    fn profile_urls_are_percent_encoded() {
        assert_eq!(
            file_url(Path::new(r"C:\Users\Ayşe K\data\lo")),
            "file:///C:/Users/Ay%C5%9Fe%20K/data/lo"
        );
    }

    #[test]
    fn bytes_must_match_the_extension() {
        let d = tempfile::tempdir().unwrap();
        let f = |name: &str, bytes: &[u8]| {
            let p = d.path().join(name);
            fs::write(&p, bytes).unwrap();
            p
        };
        let ok = f("a.docx", b"PK\x03\x04rest");
        assert!(check_magic(&ok, Format::Docx).is_ok());
        let locked = f("b.docx", &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
        assert!(matches!(
            check_magic(&locked, Format::Docx),
            Err(ConvertError::Protected)
        ));
        let junk = f("c.xlsx", b"garbage");
        assert!(matches!(
            check_magic(&junk, Format::Xlsx),
            Err(ConvertError::Corrupt)
        ));
        assert!(check_magic(&f("d.rtf", b"{\\rtf1 x}"), Format::Rtf).is_ok());
        assert!(check_magic(&f("e.rtf", b"plain"), Format::Rtf).is_err());
        // legacy formats and text are left to LibreOffice's own detection
        assert!(check_magic(&f("f.doc", b"<html>"), Format::Doc).is_ok());
        assert!(check_magic(&f("g.csv", b"a,b"), Format::Csv).is_ok());
    }

    #[test]
    fn every_office_target_has_its_own_filter() {
        for t in Format::ALL.into_iter().filter(|t| t.is_office()) {
            assert!(export_filter(t).starts_with(t.ext()), "{t:?}");
        }
        assert_eq!(export_filter(Format::Pdf), "pdf");
    }
}
