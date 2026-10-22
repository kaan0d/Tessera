//! Self-installer: the release exe is its own setup. `setup_mode` decides at launch, the
//! installer window (`install.html`) drives `install_start`, `--uninstall` undoes it.
//! Everything is per-user (HKCU, %LOCALAPPDATA%, %APPDATA%): no admin rights.

use std::fs;
use std::io::{self, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use winreg::enums::{HKEY_CURRENT_USER, KEY_ALL_ACCESS};
use winreg::RegKey;

use crate::app_info::APP_NAME;
use crate::cli::Cli;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Tessera";
const EXE_NAME: &str = "tessera.exe";
/// Each step shows for at least this long, so the screen reads as progress, not a flash.
const MIN_STEP: Duration = Duration::from_millis(700);

#[derive(Serialize)]
pub struct Defaults {
    pub mode: &'static str,
    pub dir: String,
    pub installed: bool,
    pub portable: bool,
}

#[derive(Deserialize, Debug)]
pub struct Request {
    pub dir: String,
    pub portable: bool,
}

#[derive(Serialize, Clone)]
pub struct Progress {
    pub p: f32,
    pub label: &'static str,
    pub step: u8,
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase().trim_end_matches('\\')
        == b.to_string_lossy().to_lowercase().trim_end_matches('\\')
}

/// Folder recorded by the last install (`InstallLocation` of the uninstall entry).
pub fn installed_dir() -> Option<PathBuf> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(UNINSTALL_KEY)
        .ok()?;
    let dir: String = key.get_value("InstallLocation").ok()?;
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Install,
    Uninstall,
}

/// Which setup window a launch opens, if any. A plain launch of an exe that is not the
/// installed copy opens the installer; debug builds skip that (`--setup` forces it) so the
/// dev workflow keeps starting the app. `--uninstall` always shows its window unless `--quiet`.
pub fn setup_mode(cmd: &Cli) -> Option<Mode> {
    match cmd {
        Cli::Setup => Some(Mode::Install),
        Cli::Uninstall { quiet: false, .. } => Some(Mode::Uninstall),
        Cli::Normal if !cfg!(debug_assertions) => {
            let here = exe_dir();
            let installed =
                matches!((here, installed_dir()), (Some(h), Some(i)) if same_path(&h, &i));
            (!installed).then_some(Mode::Install)
        }
        _ => None,
    }
}

pub fn default_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_NAME)
}

pub fn defaults(mode: Mode) -> Defaults {
    let installed = installed_dir().filter(|d| d.join(EXE_NAME).is_file());
    let dir = installed.clone().unwrap_or_else(default_dir);
    Defaults {
        mode: if mode == Mode::Uninstall {
            "uninstall"
        } else {
            "install"
        },
        portable: dir.join("data").is_dir(),
        installed: installed.is_some(),
        dir: dir.to_string_lossy().into_owned(),
    }
}

/// The folder the app goes into: always ends in `Tessera`, so uninstalling can never touch
/// a folder the user shares with other things (`D:\Apps` becomes `D:\Apps\Tessera`).
pub fn normalize_dir(raw: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(raw.trim().trim_matches('"'));
    if raw.trim().is_empty() || !p.is_absolute() {
        return Err(
            "Pick a full folder path, like C:\\Users\\you\\AppData\\Local\\Tessera.".into(),
        );
    }
    if crate::convert::security::is_remote_or_device(&p)
        || p.components().any(|c| matches!(c, Component::ParentDir))
    {
        return Err("Pick a folder on a local drive.".into());
    }
    let last = p.file_name().map(|n| n.to_string_lossy().to_lowercase());
    Ok(if last.as_deref() == Some("tessera") {
        p
    } else {
        p.join(APP_NAME)
    })
}

/// Stops other running copies (the installed app holds its exe open); never this process.
fn stop_running() {
    let _ = Command::new("taskkill")
        .args(["/F", "/IM", EXE_NAME, "/FI"])
        .arg(format!("PID ne {}", std::process::id()))
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    std::thread::sleep(Duration::from_millis(300));
}

fn copy_with_progress(src: &Path, dst: &Path, mut on: impl FnMut(f32)) -> io::Result<()> {
    let total = fs::metadata(src)?.len().max(1);
    let tmp = dst.with_extension("exe.new");
    let mut r = fs::File::open(src)?;
    let mut w = fs::File::create(&tmp)?;
    let (mut buf, mut done) = (vec![0u8; 256 * 1024], 0u64);
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        w.write_all(&buf[..n])?;
        done += n as u64;
        on(done as f32 / total as f32);
    }
    w.sync_all()?;
    drop(w);
    fs::rename(&tmp, dst)
}

/// One install step: gets a callback for its own 0..1 progress.
type Work<'a> = &'a mut dyn FnMut(&mut dyn FnMut(f32)) -> Result<(), String>;

fn fail(what: &'static str) -> impl Fn(io::Error) -> String {
    move |e| format!("{what}: {e}")
}

fn start_menu_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(r"Microsoft\Windows\Start Menu\Programs")
}

fn write_uninstall_entry(dir: &Path, exe: &Path) -> io::Result<()> {
    let (k, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey_with_flags(UNINSTALL_KEY, KEY_ALL_ACCESS)?;
    let size_kb = fs::metadata(exe).map_or(0, |m| (m.len() / 1024) as u32);
    let uninstall = format!(r#""{}" --uninstall"#, exe.display());
    let quiet = format!(r#""{}" --uninstall --quiet"#, exe.display());
    k.set_value("DisplayName", &APP_NAME.to_string())?;
    k.set_value("DisplayVersion", &env!("CARGO_PKG_VERSION").to_string())?;
    k.set_value("Publisher", &"kaan0d".to_string())?;
    k.set_value("InstallLocation", &dir.to_string_lossy().to_string())?;
    k.set_value("DisplayIcon", &exe.to_string_lossy().to_string())?;
    k.set_value("UninstallString", &uninstall)?;
    k.set_value("QuietUninstallString", &quiet)?;
    k.set_value("EstimatedSize", &size_kb)?;
    k.set_value("NoModify", &1u32)?;
    k.set_value("NoRepair", &1u32)
}

/// One step with a minimum on-screen time; `work` reports its own 0..1 progress.
fn run_step(
    emit: &mut dyn FnMut(Progress),
    n: u8,
    label: &'static str,
    (from, to): (f32, f32),
    work: Work,
) -> Result<(), String> {
    let t0 = Instant::now();
    emit(Progress {
        p: from,
        label,
        step: n,
    });
    let r = {
        let mut sub = |f: f32| {
            emit(Progress {
                p: from + (to - from) * f.clamp(0.0, 1.0),
                label,
                step: n,
            })
        };
        work(&mut sub)
    };
    if let Some(rest) = MIN_STEP.checked_sub(t0.elapsed()) {
        std::thread::sleep(rest);
    }
    emit(Progress {
        p: to,
        label,
        step: n,
    });
    r
}

/// Runs the install, reporting each step through `emit`. Returns the installed exe.
pub fn run_install(req: &Request, mut emit: impl FnMut(Progress)) -> Result<PathBuf, String> {
    let dir = normalize_dir(&req.dir)?;
    let src = std::env::current_exe().map_err(|e| e.to_string())?;
    let dst = dir.join(EXE_NAME);
    let emit: &mut dyn FnMut(Progress) = &mut emit;

    run_step(emit, 1, "Copying files", (0.0, 0.55), &mut |sub| {
        stop_running();
        fs::create_dir_all(&dir).map_err(fail("Cannot create the folder"))?;
        if req.portable {
            fs::create_dir_all(dir.join("data")).map_err(fail("Cannot create the data folder"))?;
        }
        if !same_path(&src, &dst) {
            copy_with_progress(&src, &dst, &mut *sub).map_err(fail("Cannot copy the app"))?;
        }
        Ok(())
    })?;
    run_step(emit, 2, "Registering the menu", (0.55, 0.78), &mut |_| {
        let ok = Command::new(&dst)
            .arg("--install-menu")
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(fail("Cannot register the menu"))?
            .success();
        ok.then_some(())
            .ok_or_else(|| "Cannot register the menu.".to_string())
    })?;
    run_step(
        emit,
        3,
        "Adding icons and shortcuts",
        (0.78, 0.93),
        &mut |_| {
            crate::shell::set_sendto_at(&start_menu_dir(), &dst, true)
                .map_err(fail("Cannot add the Start menu shortcut"))?;
            write_uninstall_entry(&dir, &dst).map_err(fail("Cannot register the uninstaller"))
        },
    )?;
    run_step(emit, 4, "Finishing up", (0.93, 1.0), &mut |_| Ok(()))?;
    Ok(dst)
}

/// The folder this exe runs from, when it is the installed copy (and so safe to delete).
fn installed_self() -> Result<(PathBuf, PathBuf), String> {
    let dir = exe_dir().ok_or("Cannot find the app folder.")?;
    let ours = installed_dir().is_some_and(|d| same_path(&d, &dir))
        && dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("tessera"));
    // refuse to delete a folder we did not create
    ours.then(|| (dir.join(EXE_NAME), dir))
        .ok_or_else(|| "This is not an installed copy of Tessera.".into())
}

/// Undoes everything `run_install` did. Failures of single removals are ignored on purpose:
/// a half-removed app is worse than one with a stray key. `keep_data` leaves settings, history
/// and the downloaded ffmpeg in place.
pub fn run_uninstall(keep_data: bool, mut emit: impl FnMut(Progress)) -> Result<(), String> {
    let (exe, dir) = installed_self()?;
    let emit: &mut dyn FnMut(Progress) = &mut emit;
    run_step(emit, 1, "Closing Tessera", (0.0, 0.2), &mut |_| {
        stop_running();
        Ok(())
    })?;
    run_step(emit, 2, "Removing the menu", (0.2, 0.5), &mut |_| {
        let _ = crate::shell::set_autostart(false);
        let _ = crate::shell::remove();
        Ok(())
    })?;
    run_step(
        emit,
        3,
        "Removing shortcuts and entries",
        (0.5, 0.75),
        &mut |_| {
            let _ = crate::shell::set_sendto_at(&start_menu_dir(), Path::new(""), false);
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(UNINSTALL_KEY);
            Ok(())
        },
    )?;
    run_step(emit, 4, "Deleting files", (0.75, 1.0), &mut |_| {
        if !keep_data {
            let _ = fs::remove_dir_all(crate::settings::data_dir());
        }
        schedule_cleanup(&exe, &dir)
    })
}

/// The exe cannot delete itself while it runs: a detached `cmd` waits (up to a minute, the
/// uninstall window may stay open) until the exe is gone, then removes the empty folder and
/// the WebView2 cache the app created in %LOCALAPPDATA%.
fn schedule_cleanup(exe: &Path, dir: &Path) -> Result<(), String> {
    let web = std::env::var_os("LOCALAPPDATA")
        .map(|b| PathBuf::from(b).join("com.tessera.app"))
        .unwrap_or_default();
    // raw_arg: cmd does its own quote parsing and rejects the backslash-escaped quotes `arg` adds
    let script = format!(
        r#"/C "for /l %i in (1,1,60) do (if exist "{exe}" (del /f /q "{exe}" 2>nul & ping -n 2 127.0.0.1 >nul)) & rmdir "{dir}" & rmdir /s /q "{web}"""#,
        exe = exe.display(),
        dir = dir.display(),
        web = web.display()
    );
    Command::new("cmd")
        .raw_arg(script)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(fail("Cannot finish the removal"))
}

/// `--uninstall --quiet`: no window, exit code only.
pub fn uninstall_quiet(keep_data: bool) -> i32 {
    i32::from(run_uninstall(keep_data, |_| {}).is_err())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_always_ends_in_tessera() {
        let n = |s: &str| normalize_dir(s).unwrap();
        assert_eq!(n(r"D:\Apps"), PathBuf::from(r"D:\Apps\Tessera"));
        assert_eq!(n(r"D:\Apps\tessera"), PathBuf::from(r"D:\Apps\tessera"));
        assert_eq!(
            n(r#" "D:\Apps\Tessera" "#),
            PathBuf::from(r"D:\Apps\Tessera")
        );
    }

    #[test]
    fn bad_folders_are_refused() {
        for bad in ["", "tessera", r"..\x", r"\\server\share\x", r"D:\a\..\b"] {
            assert!(normalize_dir(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn paths_compare_without_case_or_trailing_slash() {
        assert!(same_path(
            Path::new(r"C:\A\Tessera\"),
            Path::new(r"c:\a\tessera")
        ));
        assert!(!same_path(Path::new(r"C:\A"), Path::new(r"C:\B")));
    }
}
