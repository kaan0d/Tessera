//! Self-installer: the release exe is its own setup. `setup_mode` decides at launch, the
//! installer window (`install.html`) drives `install_start`, `--uninstall` undoes it.
//! Everything is per-user (HKCU, one folder under %LOCALAPPDATA%): no admin rights.

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
const DETACHED_PROCESS: u32 = 0x0000_0008;
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Tessera";
const EXE_NAME: &str = "tessera.exe";
/// The same program flagged as a console app, for scripts: PowerShell waits for it and sees its exit code.
pub const CLI_NAME: &str = "tessera-cli.exe";
/// Each step shows for at least this long, so the screen reads as progress, not a flash.
const MIN_STEP: Duration = Duration::from_millis(500);

#[derive(Serialize)]
pub struct Defaults {
    pub mode: &'static str,
    pub lang: &'static str,
    pub dir: String,
    pub installed: bool,
    /// A working ffmpeg is already in the installed copy's data folder.
    pub ffmpeg: bool,
    /// Windows 11: the classic menu sits under "Show more options".
    pub win11: bool,
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, Debug)]
pub struct Request {
    pub dir: String,
    /// Download ffmpeg during the install (the default); false = later.
    #[serde(default = "yes")]
    pub ffmpeg: bool,
    /// Add the install folder to the user's PATH so `tessera-cli` works in a terminal.
    #[serde(default)]
    pub path: bool,
    /// In-app update: ffmpeg is left as it is and the app starts when the install is done.
    #[serde(default)]
    pub update: bool,
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

/// The folder this exe runs from when it is the installed copy.
pub fn installed_here() -> Option<PathBuf> {
    let here = exe_dir()?;
    installed_dir().filter(|i| same_path(i, &here))?;
    Some(here)
}

/// Runs the downloaded installer `setup` as a silent update of the install in `dir`, keeping
/// its PATH entry. The installer closes this app, copies itself over and restarts it.
pub fn launch_update(setup: &Path, dir: &Path) -> io::Result<()> {
    let mut cmd = Command::new(setup);
    cmd.arg("--install").arg(dir).arg("--update");
    let on_path = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(ENV_KEY)
        .map(|k| read_user_path(&k).0)
        .is_ok_and(|cur| path_with(&cur, &dir.to_string_lossy()).is_none());
    if on_path {
        cmd.arg("--add-to-path");
    }
    cmd.creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map(drop)
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

/// A working ffmpeg already sits in the install's data folder, or in the v0.1.x one it moves from.
pub fn ffmpeg_present(dir: &Path) -> bool {
    let legacy = crate::settings::legacy_data_dir().filter(|_| !dir.join("data").is_dir());
    [Some(dir.join("data")), legacy]
        .into_iter()
        .flatten()
        .any(|d| crate::ffmpeg::is_working(&d.join("ffmpeg").join("ffmpeg.exe")))
}

/// Moves every entry of `from` into `to` (rename, or copy across drives), then removes `from`.
/// Entries already in `to` stay where they are and keep `from` alive.
fn move_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let dst = to.join(e.file_name());
        if dst.exists() || fs::rename(e.path(), &dst).is_ok() {
            continue;
        }
        if e.file_type()?.is_dir() {
            move_tree(&e.path(), &dst)?;
        } else {
            fs::copy(e.path(), &dst)?;
            fs::remove_file(e.path())?;
        }
    }
    fs::remove_dir(from)
}

/// v0.1.x kept data in %APPDATA%\Tessera and the WebView2 cache in %LOCALAPPDATA%\com.tessera.app.
/// Both move into the install's `data` folder once; a folder that already has settings wins.
fn adopt_legacy_data(data: &Path) {
    if let Some(old) = crate::settings::legacy_data_dir()
        .filter(|o| o.is_dir() && !data.join("settings.json").exists())
    {
        if let Err(e) = move_tree(&old, data) {
            crate::log::write("warn", &format!("old data not fully moved: {e}"));
        }
    }
    // only a cache: the app rebuilds it in data\webview
    if let Some(web) = legacy_webview_dir() {
        let _ = fs::remove_dir_all(web);
    }
}

fn legacy_webview_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|b| PathBuf::from(b).join("com.tessera.app"))
}

fn is_win11() -> bool {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber"))
        .ok()
        .and_then(|b| b.parse::<u32>().ok())
        .is_some_and(|b| b >= 22000)
}

pub fn defaults(mode: Mode) -> Defaults {
    let installed = installed_dir().filter(|d| d.join(EXE_NAME).is_file());
    let dir = installed.clone().unwrap_or_else(default_dir);
    let ffmpeg = installed.is_some() && ffmpeg_present(&dir);
    Defaults {
        ffmpeg,
        win11: is_win11(),
        lang: if crate::i18n::system_lang() == crate::i18n::Lang::Tr {
            "tr"
        } else {
            "en"
        },
        mode: if mode == Mode::Uninstall {
            "uninstall"
        } else {
            "install"
        },
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
    if p.to_string_lossy()
        .chars()
        .any(|c| matches!(c, '%' | '^' | '&' | '!' | '"' | '<' | '>' | '|'))
    {
        return Err("The folder name cannot contain % ^ & ! < > | or quotes.".into());
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

/// Process ids whose image is one of `targets` (never this process).
fn running_copies(targets: &[PathBuf]) -> Vec<u32> {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    let me = std::process::id();
    let mut pids = Vec::new();
    // SAFETY: plain Win32 calls on handles opened and closed right here; the buffers are local.
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return pids;
        };
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut more = Process32FirstW(snap, &mut e).is_ok();
        while more {
            if e.th32ProcessID != me {
                if let Ok(h) =
                    OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, e.th32ProcessID)
                {
                    let mut buf = [0u16; 520];
                    let mut len = buf.len() as u32;
                    if QueryFullProcessImageNameW(
                        h,
                        PROCESS_NAME_FORMAT(0),
                        PWSTR(buf.as_mut_ptr()),
                        &mut len,
                    )
                    .is_ok()
                    {
                        let path = PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]));
                        if targets.iter().any(|t| same_path(t, &path)) {
                            pids.push(e.th32ProcessID);
                        }
                    }
                    let _ = CloseHandle(h);
                }
            }
            more = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    pids
}

/// Stops the running copies of the app that live in `dir` (the installed exe holds its file
/// open). Copies run from anywhere else, like a dev build, are left alone; so is this process.
fn stop_running(dir: &Path) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
    let targets = [dir.join(EXE_NAME), dir.join(CLI_NAME)];
    let pids = running_copies(&targets);
    for pid in &pids {
        // SAFETY: the handle is opened and closed right here.
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, *pid) {
                let _ = TerminateProcess(h, 1);
                let _ = CloseHandle(h);
            }
        }
    }
    if !pids.is_empty() {
        std::thread::sleep(Duration::from_millis(400));
    }
}

/// A copy of an exe image with the PE subsystem set to console (3) instead of GUI (2).
/// None when the bytes are not a PE file.
pub fn console_copy_bytes(mut bytes: Vec<u8>) -> Option<Vec<u8>> {
    let pe = u32::from_le_bytes(bytes.get(0x3c..0x40)?.try_into().ok()?) as usize;
    if bytes.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    // optional header starts 24 bytes into the PE header; Subsystem sits 68 bytes in (PE32 and PE32+)
    let slot = bytes.get_mut(pe + 24 + 68..pe + 24 + 70)?;
    slot.copy_from_slice(&3u16.to_le_bytes());
    Some(bytes)
}

/// Writes `tessera-cli.exe` next to the app (a console-flagged copy of the exe).
fn write_cli_copy(exe: &Path, dir: &Path) -> io::Result<()> {
    let patched =
        console_copy_bytes(fs::read(exe)?).ok_or_else(|| io::Error::other("not a PE file"))?;
    let tmp = dir.join("tessera-cli.exe.new");
    fs::write(&tmp, patched)?;
    fs::rename(tmp, dir.join(CLI_NAME))
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

const ENV_KEY: &str = "Environment";

/// Entries are equal when they differ only in case, spaces or a trailing backslash.
fn same_entry(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches(['\\', '/']).to_lowercase();
    norm(a) == norm(b)
}

/// `path` with `dir` appended; None when it is already listed.
pub fn path_with(path: &str, dir: &str) -> Option<String> {
    if path.split(';').any(|e| same_entry(e, dir)) {
        return None;
    }
    Some(if path.trim().is_empty() {
        dir.to_string()
    } else if path.ends_with(';') {
        format!("{path}{dir}")
    } else {
        format!("{path};{dir}")
    })
}

/// `path` without `dir`; None when it was not listed. Every other entry stays as it was.
pub fn path_without(path: &str, dir: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(';').collect();
    if !parts.iter().any(|e| same_entry(e, dir)) {
        return None;
    }
    let kept: Vec<&str> = parts.into_iter().filter(|e| !same_entry(e, dir)).collect();
    Some(kept.join(";"))
}

/// The user's PATH exactly as stored, with its registry type (so %VARS% survive a rewrite).
fn read_user_path(key: &RegKey) -> (String, winreg::enums::RegType) {
    match key.get_raw_value("Path") {
        Ok(v) => {
            let units: Vec<u16> = v
                .bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect();
            let text = String::from_utf16_lossy(&units);
            (text.trim_end_matches('\0').to_string(), v.vtype)
        }
        Err(_) => (String::new(), winreg::enums::REG_EXPAND_SZ),
    }
}

fn write_user_path(key: &RegKey, value: &str, vtype: winreg::enums::RegType) -> io::Result<()> {
    let bytes: Vec<u8> = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(|u| u.to_le_bytes())
        .collect();
    key.set_raw_value("Path", &winreg::RegValue { bytes, vtype })
}

/// Tells running programs (Explorer, new terminals) that the environment changed.
fn broadcast_environment_change() {
    use windows::core::w;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    };
    // SAFETY: the string literal outlives the call; the result is not used.
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("Environment").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            3000,
            None,
        );
    }
}

/// Adds the install folder to the user's PATH (no admin rights, once).
pub fn add_to_path(dir: &Path) -> io::Result<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(ENV_KEY, KEY_ALL_ACCESS)?;
    let (cur, vtype) = read_user_path(&key);
    if let Some(new) = path_with(&cur, &dir.to_string_lossy()) {
        write_user_path(&key, &new, vtype)?;
        broadcast_environment_change();
    }
    Ok(())
}

/// Takes the install folder out of the user's PATH again; nothing else is touched.
pub fn remove_from_path(dir: &Path) -> io::Result<()> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(ENV_KEY, KEY_ALL_ACCESS)?;
    let (cur, vtype) = read_user_path(&key);
    if let Some(new) = path_without(&cur, &dir.to_string_lossy()) {
        write_user_path(&key, &new, vtype)?;
        broadcast_environment_change();
    }
    Ok(())
}

/// What an install produced: the exe, and whether video support (ffmpeg) is in place.
pub struct Installed {
    pub exe: PathBuf,
    /// Some(true) downloaded, Some(false) failed, None skipped on purpose.
    pub ffmpeg: Option<bool>,
}

/// Runs `<exe> --install-ffmpeg`, mapping its `P <percent>` lines onto `sub`. True on success.
fn download_ffmpeg(exe: &Path, sub: &mut dyn FnMut(f32)) -> bool {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let Ok(mut child) = Command::new(exe)
        .arg("--install-ffmpeg")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
    else {
        return false;
    };
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(p) = line
                .strip_prefix("P ")
                .and_then(|p| p.trim().parse::<f32>().ok())
            {
                sub(p / 100.0);
            }
        }
    }
    child.wait().is_ok_and(|s| s.success())
}

/// Runs the install, reporting each step through `emit`.
pub fn run_install(req: &Request, mut emit: impl FnMut(Progress)) -> Result<Installed, String> {
    let dir = normalize_dir(&req.dir)?;
    let src = std::env::current_exe().map_err(|e| e.to_string())?;
    let dst = dir.join(EXE_NAME);
    let emit: &mut dyn FnMut(Progress) = &mut emit;
    let mut ffmpeg_ok: Option<bool> = None;

    run_step(emit, 1, "Copying files", (0.0, 0.06), &mut |sub| {
        stop_running(&dir);
        fs::create_dir_all(&dir).map_err(fail("Cannot create the folder"))?;
        adopt_legacy_data(&dir.join("data"));
        fs::create_dir_all(dir.join("data")).map_err(fail("Cannot create the data folder"))?;
        if !same_path(&src, &dst) {
            copy_with_progress(&src, &dst, &mut *sub).map_err(fail("Cannot copy the app"))?;
        }
        // for scripts; the app itself works without it
        if let Err(e) = write_cli_copy(&dst, &dir) {
            crate::log::write("warn", &format!("command-line copy not written: {e}"));
        }
        Ok(())
    })?;
    run_step(emit, 2, "Registering the menu", (0.06, 0.09), &mut |_| {
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
        (0.09, 0.12),
        &mut |_| {
            crate::shell::set_sendto_at(&start_menu_dir(), &dst, true)
                .map_err(fail("Cannot add the Start menu shortcut"))?;
            if req.path {
                if let Err(e) = add_to_path(&dir) {
                    crate::log::write("warn", &format!("PATH not updated: {e}"));
                }
            }
            write_uninstall_entry(&dir, &dst).map_err(fail("Cannot register the uninstaller"))
        },
    )?;
    // not fatal: offline, the app downloads it itself the next time it starts
    run_step(
        emit,
        4,
        if req.ffmpeg {
            "Downloading video support"
        } else {
            "Skipping video support"
        },
        (0.12, 0.96),
        &mut |sub| {
            if req.update {
            } else if req.ffmpeg {
                ffmpeg_ok = Some(download_ffmpeg(&dst, sub));
            } else {
                // the app then waits for the user instead of fetching 170 MB on its own
                let _ = Command::new(&dst)
                    .arg("--defer-ffmpeg")
                    .creation_flags(CREATE_NO_WINDOW)
                    .status();
            }
            Ok(())
        },
    )?;
    run_step(emit, 5, "Finishing up", (0.96, 1.0), &mut |_| {
        if req.update {
            Command::new(&dst)
                .creation_flags(DETACHED_PROCESS)
                .spawn()
                .map_err(fail("Cannot start the app"))?;
        }
        Ok(())
    })?;
    Ok(Installed {
        exe: dst,
        ffmpeg: ffmpeg_ok,
    })
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
pub fn run_uninstall(
    keep_data: bool,
    mut emit: impl FnMut(Progress),
) -> Result<Vec<&'static str>, String> {
    let (exe, dir) = installed_self()?;
    let emit: &mut dyn FnMut(Progress) = &mut emit;
    // what could not be removed, by name; the last screen lists it
    let mut left: Vec<&'static str> = Vec::new();
    let done =
        |r: io::Result<()>| r.is_ok() || r.is_err_and(|e| e.kind() == io::ErrorKind::NotFound);
    run_step(emit, 1, "Closing Tessera", (0.0, 0.2), &mut |_| {
        stop_running(&dir);
        Ok(())
    })?;
    run_step(emit, 2, "Removing the menu", (0.2, 0.5), &mut |_| {
        if !done(crate::shell::set_autostart(false)) {
            left.push("autostart");
        }
        if !done(crate::shell::remove()) {
            left.push("menu");
        }
        Ok(())
    })?;
    run_step(
        emit,
        3,
        "Removing shortcuts and entries",
        (0.5, 0.75),
        &mut |_| {
            if !done(crate::shell::set_sendto_at(
                &start_menu_dir(),
                Path::new(""),
                false,
            )) {
                left.push("shortcut");
            }
            if !done(RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(UNINSTALL_KEY)) {
                left.push("entry");
            }
            if !done(remove_from_path(&dir)) {
                left.push("path");
            }
            Ok(())
        },
    )?;
    run_step(emit, 4, "Deleting files", (0.75, 1.0), &mut |_| {
        if !keep_data && !done(fs::remove_dir_all(crate::settings::data_dir())) {
            left.push("data");
        }
        if schedule_cleanup(&exe, &dir, keep_data).is_err() {
            left.push("files");
        }
        Ok(())
    })?;
    Ok(left)
}

/// The exe cannot delete itself while it runs: a detached `cmd` waits (up to a minute, the
/// uninstall window may stay open) until the exe is gone, then removes the WebView2 cache, the
/// data folder unless it is kept, and the app folder once it is empty.
fn schedule_cleanup(exe: &Path, dir: &Path, keep_data: bool) -> Result<(), String> {
    let data = dir.join("data");
    // the app's WebView2 processes may still have held files when step 4 ran
    let wipe = if keep_data {
        data.join("webview")
    } else {
        data
    };
    let legacy = legacy_webview_dir().unwrap_or_default();
    // raw_arg: cmd does its own quote parsing and rejects the backslash-escaped quotes `arg` adds
    let script = format!(
        r#"/C "for /l %i in (1,1,60) do (if exist "{exe}" (del /f /q "{exe}" 2>nul & ping -n 2 127.0.0.1 >nul)) & del /f /q "{cli}" 2>nul & rmdir /s /q "{wipe}" & rmdir /s /q "{legacy}" & rmdir "{dir}"""#,
        exe = exe.display(),
        cli = dir.join(CLI_NAME).display(),
        wipe = wipe.display(),
        legacy = legacy.display(),
        dir = dir.display()
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
    match run_uninstall(keep_data, |_| {}) {
        Ok(left) => i32::from(!left.is_empty()),
        Err(_) => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_copy_flips_only_the_subsystem() {
        let mut b = vec![0u8; 0x200];
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        let at = 0x80 + 24 + 68;
        b[at..at + 2].copy_from_slice(&2u16.to_le_bytes());
        let c = console_copy_bytes(b.clone()).unwrap();
        assert_eq!(&c[at..at + 2], &3u16.to_le_bytes());
        assert_eq!(c.len(), b.len());
        assert_eq!(c.iter().zip(&b).filter(|(x, y)| x != y).count(), 1);
        assert!(console_copy_bytes(vec![0; 16]).is_none());
        assert!(console_copy_bytes(vec![0; 0x200]).is_none());
    }

    #[test]
    fn old_data_moves_without_overwriting() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("old"), t.path().join("new"));
        fs::create_dir_all(old.join("ffmpeg")).unwrap();
        fs::write(old.join("settings.json"), "old").unwrap();
        fs::write(old.join("ffmpeg").join("ffmpeg.exe"), "x").unwrap();
        move_tree(&old, &new).unwrap();
        assert!(!old.exists());
        assert_eq!(
            fs::read_to_string(new.join("settings.json")).unwrap(),
            "old"
        );
        assert!(new.join("ffmpeg").join("ffmpeg.exe").is_file());

        // an entry already in the new folder wins and keeps the old folder in place
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("tessera.log"), "old").unwrap();
        fs::write(new.join("tessera.log"), "new").unwrap();
        assert!(move_tree(&old, &new).is_err());
        assert_eq!(fs::read_to_string(new.join("tessera.log")).unwrap(), "new");
    }

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

#[cfg(test)]
mod folder_name_tests {
    use super::*;

    #[test]
    fn names_cmd_would_reinterpret_are_refused() {
        for c in ['%', '^', '&', '!', '<', '>', '|', '"'] {
            let p = format!("D:/apps/a{c}b");
            assert!(normalize_dir(&p).is_err(), "{p}");
        }
        assert!(normalize_dir("D:/apps/My Tools (x86)").is_ok());
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn adding_is_idempotent_and_keeps_the_rest() {
        let dir = "C:/Apps/Tessera";
        assert_eq!(path_with("", dir).as_deref(), Some(dir));
        assert_eq!(
            path_with("C:/a;%X%/b", dir).as_deref(),
            Some("C:/a;%X%/b;C:/Apps/Tessera")
        );
        assert_eq!(
            path_with("C:/a;", dir).as_deref(),
            Some("C:/a;C:/Apps/Tessera")
        );
        // already there, any case, with a trailing slash
        assert_eq!(path_with("C:/a;c:/apps/tessera/", dir), None);
    }

    #[test]
    fn removing_touches_only_our_entry() {
        let dir = "C:/Apps/Tessera";
        assert_eq!(
            path_without("C:/a;C:/Apps/Tessera;%X%/b", dir).as_deref(),
            Some("C:/a;%X%/b")
        );
        assert_eq!(path_without("C:/Apps/Tessera", dir).as_deref(), Some(""));
        assert_eq!(path_without("C:/a;C:/Apps/Tessera2", dir), None);
    }
}
