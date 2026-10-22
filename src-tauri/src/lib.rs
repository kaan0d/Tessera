pub mod app_info;
pub mod batch;
pub mod cli;
pub mod convert;
pub mod dropped;
pub mod ffmpeg;
pub mod folder;
pub mod headless;
pub mod history;
pub mod i18n;
pub mod install;
pub mod log;
pub mod panel;
pub mod preset;
pub mod queue;
pub mod settings;
pub mod shell;
pub mod update;
pub mod watch;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use batch::Collector;
use cli::Cli;
use convert::combine::MergeKind;
use convert::Format;
use ffmpeg::{Download, FfmpegStatus};
use history::Kind;
use i18n::{fmt, t, Lang};
use queue::{Queue, Spec};
use settings::Settings;

/// Tray mark for the taskbar theme: dark tiles on a light taskbar, light tiles on a dark one.
fn tray_icon(light_taskbar: bool) -> tauri::image::Image<'static> {
    let png: &[u8] = if light_taskbar {
        include_bytes!("../icons/tray-light.png")
    } else {
        include_bytes!("../icons/tray-dark.png")
    };
    tauri::image::Image::from_bytes(png).expect("embedded tray icon is a valid PNG")
}

/// True when Windows draws the taskbar (and tray) light. Unreadable setting counts as dark.
fn taskbar_is_light() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("SystemUsesLightTheme"))
        .is_ok_and(|v| v == 1)
}

/// Swaps the tray mark when the user switches the Windows theme while the app runs.
fn watch_taskbar_theme(app: AppHandle) {
    std::thread::spawn(move || {
        let mut light = taskbar_is_light();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(10));
            let now = taskbar_is_light();
            if now != light {
                light = now;
                if let Some(t) = app.tray_by_id(TRAY_ID) {
                    let _ = t.set_icon(Some(tray_icon(now)));
                }
            }
        }
    });
}

const TRAY_ID: &str = "main";

struct AppState {
    settings: Mutex<Settings>,
    queue: Queue,
    download: Arc<Download>,
    merges: Arc<Mutex<Collector>>,
    watcher: Mutex<Option<watch::Running>>,
    /// Files the chooser window currently shows (it reads them on load; the event can be early).
    dropped: Mutex<Vec<PathBuf>>,
    /// The chooser was opened by the "Custom settings" verb: it starts on its custom page.
    drop_custom: std::sync::atomic::AtomicBool,
    /// Files of the "Custom settings" verb (one process per file) and when the last arrived.
    customs: Arc<Mutex<(Vec<PathBuf>, u64)>>,
}

/// Milliseconds on a monotonic clock, for the merge collector.
fn mono_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Persists `ffmpeg_ready` and refreshes the context menu so ffmpeg verbs appear or go.
fn set_ffmpeg_ready(app: &AppHandle, ready: bool) {
    let state = app.state::<AppState>();
    let mut s = state.settings.lock().unwrap();
    s.ffmpeg_ready = ready;
    let _ = shell::install(&s);
    let _ = settings::save_to(&settings::settings_path(), &s);
}

/// Downloads ffmpeg on a background thread; no-op while one is already running.
fn start_ffmpeg_download(app: &AppHandle) {
    let download = app.state::<AppState>().download.clone();
    if !download.begin() {
        return;
    }
    let lang = i18n::lang();
    notify(app, t(lang, "notify.dl_title"), t(lang, "notify.dl_body"));
    let app = app.clone();
    std::thread::spawn(move || {
        let result = ffmpeg::install_with_retries(&download);
        if let Err(e) = &result {
            log::write(
                "error",
                &format!("ffmpeg download failed: {}", e.message(i18n::Lang::En)),
            );
        }
        if result.is_ok() {
            set_ffmpeg_ready(&app, true);
            let lang = i18n::lang();
            notify(
                &app,
                t(lang, "notify.ready_title"),
                t(lang, "notify.ready_body"),
            );
        }
        download.finish(result.err());
    });
}

/// Startup check: a working ffmpeg is marked ready, a missing or corrupt one is downloaded.
fn ensure_ffmpeg(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let ready = app
            .state::<AppState>()
            .settings
            .lock()
            .unwrap()
            .ffmpeg_ready;
        if ffmpeg::is_working(&ffmpeg::exe_path()) {
            if !ready {
                set_ffmpeg_ready(&app, true);
            }
            return;
        }
        if ready {
            set_ffmpeg_ready(&app, false);
        }
        start_ffmpeg_download(&app);
    });
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn set_settings(
    settings: Settings,
    app: AppHandle,
    state: State<AppState>,
) -> Result<Settings, String> {
    settings.validate()?;
    let mut cur = state.settings.lock().unwrap();
    let mut new = settings.normalized();
    new.ffmpeg_ready = cur.ffmpeg_ready;
    // presets are changed through their own commands
    new.presets = cur.presets.clone();
    new.hidden_presets = cur.hidden_presets.clone();
    // watch rules are changed through set_watch
    new.watch = cur.watch.clone();
    new.settings_version = settings::SETTINGS_VERSION;
    let lang = new.language;
    let msg = |key, e: std::io::Error| fmt(lang, key, &[("err", &e.to_string())]);
    shell::set_autostart(new.autostart).map_err(|e| msg("cmd.autostart", e))?;
    // also re-registers the menu with labels in the new language
    shell::install(&new).map_err(|e| msg("cmd.menu_save", e))?;
    settings::save_to(&settings::settings_path(), &new).map_err(|e| msg("cmd.save", e))?;
    *state.queue.panel.lock().unwrap() = new.panel;
    state.queue.history.lock().unwrap().enabled = new.history_enabled;
    let changed = cur.language != lang;
    *cur = new.clone();
    drop(cur);
    if changed {
        i18n::set_lang(lang);
        let _ = set_tray_menu(&app, lang);
        let _ = app.emit("language-changed", lang.code());
        // status errors are localized
        restart_watch(&app);
    }
    Ok(new)
}

#[derive(Serialize)]
struct PresetInfo {
    id: String,
    /// Display name in the current language.
    name: String,
    builtin: bool,
    custom_name: Option<String>,
    kinds: Vec<String>,
    /// Extension or "same".
    target: String,
    video_target: Option<String>,
    options: preset::PresetOptions,
    hidden: bool,
}

fn preset_infos(s: &Settings) -> Vec<PresetInfo> {
    preset::all(s)
        .into_iter()
        .map(|p| PresetInfo {
            name: p.name(s.language),
            builtin: p.name_key.is_some(),
            hidden: s.hidden_presets.contains(&p.id),
            custom_name: p.custom_name,
            kinds: p.kinds,
            target: p.target,
            video_target: p.video_target,
            options: p.options,
            id: p.id,
        })
        .collect()
}

/// Saves settings and re-registers the menu after a preset change.
fn persist_presets(s: &Settings) -> Result<(), String> {
    let lang = s.language;
    let msg = |key, e: std::io::Error| fmt(lang, key, &[("err", &e.to_string())]);
    shell::install(s).map_err(|e| msg("cmd.menu_save", e))?;
    settings::save_to(&settings::settings_path(), s).map_err(|e| msg("cmd.save", e))
}

#[tauri::command]
fn list_presets(state: State<AppState>) -> Vec<PresetInfo> {
    preset_infos(&state.settings.lock().unwrap())
}

/// Adds or replaces a custom preset (empty `id` makes a new one). Returns the full list.
#[tauri::command]
fn save_preset(preset: preset::Preset, state: State<AppState>) -> Result<Vec<PresetInfo>, String> {
    let mut s = state.settings.lock().unwrap();
    let lang = s.language;
    let mut p = preset;
    if p.id.trim().is_empty() {
        p.id = format!("c{}", history::now_ms());
    }
    p.validate(lang)?;
    let mut new = s.clone();
    if let Some(i) = new.presets.iter().position(|x| x.id == p.id) {
        new.presets[i] = p;
    } else if new.presets.len() >= preset::MAX_CUSTOM {
        return Err(preset::too_many(lang));
    } else {
        new.presets.push(p);
    }
    persist_presets(&new)?;
    *s = new;
    Ok(preset_infos(&s))
}

/// Deletes a custom preset; built-ins are refused (hide them instead). Returns the full list.
#[tauri::command]
fn delete_preset(id: String, state: State<AppState>) -> Result<Vec<PresetInfo>, String> {
    let mut s = state.settings.lock().unwrap();
    let lang = s.language;
    if preset::is_builtin(&id) {
        return Err(t(lang, "preset.builtin").into());
    }
    let mut new = s.clone();
    let before = new.presets.len();
    new.presets.retain(|p| p.id != id);
    if new.presets.len() == before {
        return Err(t(lang, "preset.not_found").into());
    }
    new.hidden_presets.retain(|h| h != &id);
    persist_presets(&new)?;
    *s = new;
    Ok(preset_infos(&s))
}

/// Hides or shows a preset (built-in or custom) in the context menu. Returns the full list.
#[tauri::command]
fn set_preset_hidden(
    id: String,
    hidden: bool,
    state: State<AppState>,
) -> Result<Vec<PresetInfo>, String> {
    let mut s = state.settings.lock().unwrap();
    if preset::find(&s, &id).is_none() {
        return Err(t(s.language, "preset.not_found").into());
    }
    let mut new = s.clone();
    new.hidden_presets.retain(|h| h != &id);
    if hidden {
        new.hidden_presets.push(id);
    }
    persist_presets(&new)?;
    *s = new;
    Ok(preset_infos(&s))
}

/// History, newest first.
#[tauri::command]
fn get_history(state: State<AppState>) -> Vec<history::Entry> {
    state.queue.history.lock().unwrap().list()
}

#[tauri::command]
fn clear_history(app: AppHandle, state: State<AppState>) {
    state.queue.history.lock().unwrap().clear();
    let _ = app.emit("history-changed", ());
}

/// Moves the job's output to the Recycle Bin and brings back a recycled original.
#[tauri::command]
async fn undo_job(id: u64, app: AppHandle) -> Result<(), String> {
    let history = app.state::<AppState>().queue.history.clone();
    let lang = i18n::lang();
    let r = tauri::async_runtime::spawn_blocking(move || history::undo(&history, id, lang))
        .await
        .map_err(|e| e.to_string())?;
    let _ = app.emit("history-changed", ());
    r
}

#[derive(Serialize)]
struct WatchInfo {
    #[serde(flatten)]
    watch: watch::Watch,
    status: Vec<watch::RuleStatus>,
}

fn watch_info(app: &AppHandle) -> WatchInfo {
    let state = app.state::<AppState>();
    let watch = state.settings.lock().unwrap().watch.clone();
    let status = state
        .watcher
        .lock()
        .unwrap()
        .as_ref()
        .map(|r| r.status.clone())
        .unwrap_or_default();
    WatchInfo { watch, status }
}

/// (Re)starts the folder watchers from the saved settings. Pending files are dropped.
/// Never call with the settings lock held: the watcher thread takes it.
fn restart_watch(app: &AppHandle) {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().clone();
    let mut slot = state.watcher.lock().unwrap();
    if let Some(old) = slot.take() {
        old.stop();
    }
    let handle = app.clone();
    let sink: watch::Sink = Arc::new(move |rule, path| enqueue_watch(&handle, rule, path));
    *slot = Some(watch::start(
        &s.watch,
        s.language,
        sink,
        watch::Timing::default(),
    ));
}

/// A stable new file under `rule`: queued like any other job (history kind "watch").
fn enqueue_watch(app: &AppHandle, rule: &watch::WatchRule, path: PathBuf) {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().clone();
    let Some((target, opts)) = watch::plan_job(rule, &path, &s) else {
        return;
    };
    let folder = Path::new(&rule.folder)
        .file_name()
        .map_or(rule.folder.clone(), |n| n.to_string_lossy().into());
    let label = fmt(s.language, "watch.label", &[("folder", &folder)]);
    let spec = Spec {
        path,
        target,
        opts,
        merge: None,
    };
    state
        .queue
        .enqueue_specs(app, vec![spec], Kind::Watch, &label, s.ffmpeg_ready);
}

#[tauri::command]
fn get_watch(app: AppHandle) -> WatchInfo {
    watch_info(&app)
}

/// Validates, saves and restarts the watchers. Returns the saved watch config.
#[tauri::command]
fn set_watch(watch: watch::Watch, app: AppHandle) -> Result<watch::Watch, String> {
    let state = app.state::<AppState>();
    let mut cur = state.settings.lock().unwrap();
    let mut w = watch;
    w.validate(&cur, cur.language)?;
    let mut new = cur.clone();
    new.watch = w.clone();
    settings::save_to(&settings::settings_path(), &new)
        .map_err(|e| fmt(new.language, "cmd.save", &[("err", &e.to_string())]))?;
    *cur = new;
    drop(cur);
    restart_watch(&app);
    Ok(w)
}

/// Latest release check (blocking network call, so it runs on a worker thread).
#[tauri::command]
async fn check_update() -> Result<update::UpdateInfo, String> {
    let lang = i18n::lang();
    tauri::async_runtime::spawn_blocking(move || update::check(lang))
        .await
        .map_err(|e| e.to_string())?
}

/// Opens a link inside the project's GitHub repository in the default browser.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    update::open_url(&url, i18n::lang())
}

/// Once, ~10 s after launch: tells the user when a newer release exists. Silent otherwise.
fn check_update_on_start(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(10));
        let lang = i18n::lang();
        if let Ok(info) = update::check(lang) {
            if info.available {
                notify(&app, t(lang, "update.title"), &info.message);
                let _ = app.emit("update-available", &info);
            }
        }
    });
}

#[derive(Serialize)]
struct CatalogSource {
    ext: &'static str,
    kind: &'static str,
}

#[derive(Serialize)]
struct CatalogTarget {
    ext: &'static str,
    kind: &'static str,
    needs_ffmpeg: bool,
    /// Works without ffmpeg, but is lossy (better and smaller) once it is installed: WebP.
    better_with_ffmpeg: bool,
}

#[derive(Serialize)]
struct Catalog {
    sources: Vec<CatalogSource>,
    targets: Vec<CatalogTarget>,
}

/// Every source extension that can get a menu and every output format, for the settings grids.
#[tauri::command]
fn get_catalog() -> Catalog {
    Catalog {
        sources: shell::EXTS
            .iter()
            .filter_map(|e| {
                convert::Format::from_ext(e).map(|f| CatalogSource {
                    ext: e,
                    kind: f.kind(),
                })
            })
            .collect(),
        targets: convert::Format::ALL
            .iter()
            .map(|f| CatalogTarget {
                ext: f.ext(),
                kind: f.kind(),
                needs_ffmpeg: convert::needs_ffmpeg(*f),
                better_with_ffmpeg: *f == convert::Format::Webp,
            })
            .collect(),
    }
}

#[tauri::command]
fn install_context_menu(state: State<AppState>) -> Result<(), String> {
    let s = state.settings.lock().unwrap().clone();
    shell::install(&s).map_err(|e| fmt(s.language, "cmd.menu_save", &[("err", &e.to_string())]))
}

#[tauri::command]
fn remove_context_menu() -> Result<(), String> {
    shell::remove().map_err(|e| fmt(i18n::lang(), "cmd.menu_remove", &[("err", &e.to_string())]))
}

#[tauri::command]
fn cancel_job(id: u64, state: State<AppState>) {
    state.queue.cancel(id);
}

#[tauri::command]
fn ffmpeg_status(state: State<AppState>) -> FfmpegStatus {
    let ready = state.settings.lock().unwrap().ffmpeg_ready;
    state.download.status(ready, i18n::lang())
}

#[tauri::command]
fn retry_ffmpeg_download(app: AppHandle) {
    start_ffmpeg_download(&app);
}

/// Plays a fake job on the real mini panel; the UI's current (unsaved) values win over the saved ones.
#[tauri::command]
fn panel_preview(
    corner: Option<String>,
    seconds: Option<u8>,
    app: AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let mut p = *state.queue.panel.lock().unwrap();
    if let Some(c) = corner {
        p.corner = serde_json::from_value(c.into())
            .map_err(|_| t(i18n::lang(), "cmd.bad_corner").to_string())?;
    }
    if let Some(s) = seconds {
        if !(2..=30).contains(&s) {
            return Err(t(i18n::lang(), "cmd.bad_seconds").into());
        }
        p.seconds = s;
    }
    state.queue.preview(&app, p);
    Ok(())
}

/// Third-party license notices as plain text (compiled in).
#[tauri::command]
fn get_licenses() -> String {
    app_info::LICENSES.to_string()
}

/// {name, version, repo_url, license} for the About page.
#[tauri::command]
fn app_info() -> app_info::AppInfo {
    app_info::info()
}

/// Reveals a file in Explorer (or opens a folder).
#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    if path.contains('"') || convert::security::is_remote_or_device(Path::new(&path)) {
        return Err(t(i18n::lang(), "cmd.bad_path").into());
    }
    let p = Path::new(&path);
    if !p.exists() {
        return Err(t(i18n::lang(), "cmd.no_path").into());
    }
    let arg = if p.is_dir() {
        format!("\"{path}\"")
    } else {
        format!("/select,\"{path}\"")
    };
    // absolute path: a bare name would be searched in our own folder first
    let windir = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    std::process::Command::new(Path::new(&windir).join("explorer.exe"))
        .raw_arg(arg)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// Makes sure the log exists and shows it selected in Explorer (for "Open log" and bug reports).
#[tauri::command]
fn open_log() -> Result<(), String> {
    let p = log::path();
    if !p.exists() {
        log::write("info", "log opened");
    }
    open_folder(p.to_string_lossy().into_owned())
}

fn tray_menu(app: &AppHandle, lang: Lang) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(app, "open", t(lang, "tray.open"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t(lang, "tray.quit"), true, None::<&str>)?;
    Menu::with_items(app, &[&open, &quit])
}

/// Swaps the tray menu for one in `lang`.
fn set_tray_menu(app: &AppHandle, lang: Lang) -> tauri::Result<()> {
    match app.tray_by_id(TRAY_ID) {
        Some(tray) => tray.set_menu(Some(tray_menu(app, lang)?)),
        None => Ok(()),
    }
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn start_convert(app: &AppHandle, target: convert::Format, files: Vec<PathBuf>) {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().clone();
    if !s.target_enabled(target) {
        let lang = s.language;
        notify(
            app,
            t(lang, "notify.failed_title"),
            &fmt(
                lang,
                "notify.target_off",
                &[("fmt", &target.ext().to_uppercase())],
            ),
        );
        return;
    }
    state
        .queue
        .enqueue(app, target, files, &s.to_options(), s.ffmpeg_ready);
}

fn start_preset(app: &AppHandle, id: &str, files: Vec<PathBuf>) {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().clone();
    let lang = s.language;
    let Some(p) = preset::find(&s, id) else {
        notify(
            app,
            t(lang, "notify.failed_title"),
            t(lang, "preset.not_found"),
        );
        return;
    };
    run_preset(app, p, files);
}

/// Runs `p` over `files` through the queue (also used for one-off presets from the chooser).
fn run_preset(app: &AppHandle, p: preset::Preset, files: Vec<PathBuf>) {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().clone();
    let lang = s.language;
    let opts = p.options_for(&s);
    let (mut specs, mut skipped) = (vec![], 0);
    for path in files {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match p.resolve(ext, &opts) {
            Some(target) => specs.push(Spec {
                path,
                target,
                opts: opts.clone(),
                merge: None,
            }),
            None => skipped += 1,
        }
    }
    let name = p.name(lang);
    if skipped > 0 {
        let n = skipped.to_string();
        notify(
            app,
            t(lang, "notify.failed_title"),
            &fmt(lang, "notify.preset_skip", &[("n", &n), ("name", &name)]),
        );
    }
    state
        .queue
        .enqueue_specs(app, specs, Kind::Preset, &name, s.ffmpeg_ready);
}

#[derive(Serialize, Clone)]
struct DropPayload {
    files: Vec<String>,
    kinds: Vec<&'static str>,
    /// Open the custom settings page right away.
    custom: bool,
}

fn drop_payload(files: &[PathBuf], custom: bool) -> DropPayload {
    let (known, _) = dropped::split(files);
    let mut kinds: Vec<&'static str> = vec![];
    for (_, f) in known {
        if !kinds.contains(&f.kind()) {
            kinds.push(f.kind());
        }
    }
    DropPayload {
        files: files.iter().map(|f| f.display().to_string()).collect(),
        kinds,
        custom,
    }
}

/// Dropped/opened/sent files: opens the chooser window (centered, on top) with them.
/// Nothing convertible among them: a notification instead.
fn handle_drop(app: &AppHandle, paths: Vec<PathBuf>, custom: bool) {
    let files = dropped::collect(&paths);
    let lang = i18n::lang();
    if dropped::split(&files).0.is_empty() {
        notify(
            app,
            t(lang, "notify.failed_title"),
            t(lang, "drop.unsupported"),
        );
        return;
    }
    let payload = drop_payload(&files, custom);
    let state = app.state::<AppState>();
    state
        .drop_custom
        .store(custom, std::sync::atomic::Ordering::Relaxed);
    *state.dropped.lock().unwrap() = files;
    if let Some(w) = app.get_webview_window("drop") {
        let _ = w.set_always_on_top(true);
        let _ = w.center();
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        let _ = app.emit_to("drop", "drop-files", payload);
    }
}

/// Explorer starts one process per selected file; gather them (same quiet window as merge) and
/// open the chooser once, on its custom page.
fn start_custom(app: &AppHandle, files: Vec<PathBuf>) {
    let customs = app.state::<AppState>().customs.clone();
    let first = {
        let mut c = customs.lock().unwrap();
        let first = c.0.is_empty();
        c.0.extend(files);
        c.1 = mono_ms();
        first
    };
    if !first {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let ready = {
            let mut c = customs.lock().unwrap();
            (mono_ms().saturating_sub(c.1) >= batch::QUIET_MS).then(|| std::mem::take(&mut c.0))
        };
        if let Some(files) = ready {
            handle_drop(&app, files, true);
            break;
        }
    });
}

fn close_drop(app: &AppHandle) {
    app.state::<AppState>().dropped.lock().unwrap().clear();
    if let Some(w) = app.get_webview_window("drop") {
        let _ = w.hide();
    }
}

fn to_paths(files: Vec<String>) -> Vec<PathBuf> {
    files.into_iter().map(PathBuf::from).collect()
}

/// For the settings window's drag-drop event: opens the same chooser as a dropped exe launch.
#[tauri::command]
fn drop_files(paths: Vec<String>, app: AppHandle) {
    handle_drop(&app, to_paths(paths), false);
}

/// The files the chooser currently shows (call on load; `drop-files` covers later drops).
#[tauri::command]
fn get_drop_files(state: State<AppState>) -> DropPayload {
    drop_payload(
        &state.dropped.lock().unwrap(),
        state.drop_custom.load(std::sync::atomic::Ordering::Relaxed),
    )
}

#[derive(Serialize)]
struct DropOptions {
    kinds: Vec<&'static str>,
    targets: Vec<dropped::DropTarget>,
    presets: Vec<PresetInfo>,
    /// "pdf" / "audio"
    merge: Vec<&'static str>,
    count: usize,
    skipped: usize,
}

/// What the chooser may offer for these files: targets all of them share, presets that fit,
/// merges. Unknown file types are counted in `skipped`.
#[tauri::command]
fn get_drop_options(files: Vec<String>, state: State<AppState>) -> DropOptions {
    let s = state.settings.lock().unwrap().clone();
    let c = dropped::choices(&dropped::collect(&to_paths(files)), &s);
    let presets = preset_infos(&s)
        .into_iter()
        .filter(|p| c.preset_ids.contains(&p.id))
        .collect();
    DropOptions {
        kinds: c.kinds,
        targets: c.targets,
        presets,
        merge: c.merge,
        count: c.count,
        skipped: c.skipped,
    }
}

/// Runs the chooser's pick through the queue like the context menu would, then closes it.
#[tauri::command]
fn run_drop(
    files: Vec<String>,
    choice: dropped::Choice,
    app: AppHandle,
    state: State<AppState>,
) -> Result<(), String> {
    let s = state.settings.lock().unwrap().clone();
    let lang = s.language;
    let bad = || Err(t(lang, "drop.bad_choice").to_string());
    let files = dropped::collect(&to_paths(files));
    let c = dropped::choices(&files, &s);
    let known: Vec<PathBuf> = dropped::split(&files)
        .0
        .into_iter()
        .map(|(p, _)| p.clone())
        .collect();
    match choice {
        dropped::Choice::Target { target } => {
            let Some(t) = Format::from_ext(&target).filter(|f| f.ext() == target) else {
                return bad();
            };
            if !c.targets.iter().any(|x| x.ext == t.ext() && x.allowed) {
                return bad();
            }
            start_convert(&app, t, known);
        }
        dropped::Choice::Preset { id } => {
            if !c.preset_ids.contains(&id) {
                return bad();
            }
            start_preset(&app, &id, known);
        }
        dropped::Choice::Merge { merge } => {
            let Some(kind) = MergeKind::from_arg(&merge).filter(|k| c.merge.contains(&k.arg()))
            else {
                return bad();
            };
            start_merge(&app, kind, known);
        }
        dropped::Choice::Custom { target, options } => {
            let same = target == "same";
            if !same
                && !Format::from_ext(&target).is_some_and(|f| {
                    f.ext() == target && c.targets.iter().any(|x| x.ext == f.ext() && x.allowed)
                })
            {
                return bad();
            }
            let mut p = preset::Preset {
                id: "custom".into(),
                custom_name: Some(t(lang, "drop.custom_name").to_string()),
                kinds: c.kinds.iter().map(|k| k.to_string()).collect(),
                target,
                options,
                ..preset::Preset::default()
            };
            // same clamping as a saved preset; "same" without any change is refused there
            if p.validate(lang).is_err() {
                return bad();
            }
            run_preset(&app, p, known);
        }
    }
    close_drop(&app);
    Ok(())
}

/// Closes the chooser without doing anything (Escape).
#[tauri::command]
fn close_drop_window(app: AppHandle) {
    close_drop(&app);
}

/// Folder verb: converts the matching files of `dir` through the normal queue. The scan runs
/// on its own thread (a big folder takes a moment); results go through `start_convert` /
/// `start_preset`, so naming, conflicts, history and the panel behave as for the file menu.
fn start_folder(app: &AppHandle, action: folder::Action, dir: PathBuf) {
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        let s = state.settings.lock().unwrap().clone();
        let lang = s.language;
        if convert::security::is_remote_or_device(&dir) || !dir.is_dir() {
            notify(
                &app,
                t(lang, "notify.failed_title"),
                t(lang, "notify.folder_bad"),
            );
            return;
        }
        let wanted: Box<dyn Fn(&Path) -> bool> = match &action {
            folder::Action::Target(target) => {
                let (target, opts) = (*target, s.to_options());
                let ready = s.ffmpeg_ready;
                Box::new(move |p| {
                    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
                    Format::from_ext(ext).is_some_and(|f| {
                        folder::same_family(f, target)
                            && convert::allows(ext, f, target, &opts)
                            && (ready || !convert::uses_ffmpeg(f, target))
                    })
                })
            }
            folder::Action::Preset(id) => {
                let Some(p) = preset::find(&s, id) else {
                    notify(
                        &app,
                        t(lang, "notify.failed_title"),
                        t(lang, "preset.not_found"),
                    );
                    return;
                };
                let (opts, ready) = (p.options_for(&s), s.ffmpeg_ready);
                Box::new(move |path| {
                    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                    Format::from_ext(ext).is_some_and(|f| {
                        f.is_image()
                            && p.resolve(ext, &opts)
                                .is_some_and(|t| ready || !convert::uses_ffmpeg(f, t))
                    })
                })
            }
        };
        let outputs = folder::history_outputs(&state.queue.history.lock().unwrap().list());
        let own = |p: &Path| watch::is_recent(p) || folder::is_listed(&outputs, p);
        let found = folder::scan(
            &dir,
            s.menu.folder_recursive,
            folder::MAX_FILES,
            wanted,
            own,
        );
        if found.files.is_empty() {
            notify(
                &app,
                t(lang, "notify.failed_title"),
                t(lang, "notify.folder_empty"),
            );
            return;
        }
        if found.truncated {
            let max = folder::MAX_FILES.to_string();
            notify(
                &app,
                t(lang, "notify.failed_title"),
                &fmt(lang, "notify.folder_cap", &[("max", &max)]),
            );
        }
        match action {
            folder::Action::Target(target) => start_convert(&app, target, found.files),
            folder::Action::Preset(id) => start_preset(&app, &id, found.files),
        }
    });
}

/// Merge verbs arrive one process per file; they are gathered and run as one job.
fn start_merge(app: &AppHandle, kind: MergeKind, files: Vec<PathBuf>) {
    let merges = app.state::<AppState>().merges.clone();
    let first = merges.lock().unwrap().push(kind, files, mono_ms());
    if !first {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let (ready, empty) = {
            let mut c = merges.lock().unwrap();
            (c.take_ready(mono_ms()), c.is_empty())
        };
        for (kind, files) in ready {
            run_merge(&app, kind, files);
        }
        if empty {
            break;
        }
    });
}

fn run_merge(app: &AppHandle, kind: MergeKind, files: Vec<PathBuf>) {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap().clone();
    let lang = s.language;
    let label = t(
        lang,
        match kind {
            MergeKind::Pdf => "menu.merge_pdf",
            MergeKind::Audio => "menu.merge_audio",
        },
    );
    if files.len() == 1 {
        // a lone file is a plain conversion
        let target = match (kind, files[0].extension().and_then(|e| e.to_str())) {
            (MergeKind::Pdf, _) => convert::Format::Pdf,
            (_, Some(e)) if e.eq_ignore_ascii_case("flac") => convert::Format::Wav,
            _ => convert::Format::Flac,
        };
        state
            .queue
            .enqueue(app, target, files, &s.to_options(), s.ffmpeg_ready);
        return;
    }
    let spec = Spec {
        path: files[0].clone(),
        target: match kind {
            MergeKind::Pdf => convert::Format::Pdf,
            MergeKind::Audio => convert::Format::Flac,
        },
        opts: s.to_options(),
        merge: Some((kind, files)),
    };
    state
        .queue
        .enqueue_specs(app, vec![spec], Kind::Merge, label, s.ffmpeg_ready);
}

/// Headless flags used by the installer; returns the exit code.
fn run_headless(cmd: &Cli) -> Option<i32> {
    let code = |r: std::io::Result<()>| i32::from(r.is_err());
    match cmd {
        Cli::InstallMenu => {
            let s = settings::load_initial(&settings::settings_path());
            Some(code(shell::install(&s)))
        }
        Cli::InstallSendTo => Some(code(shell::set_sendto_at(
            &shell::sendto_dir(),
            &std::env::current_exe().unwrap_or_default(),
            true,
        ))),
        Cli::RemoveSendTo => Some(code(shell::set_sendto_at(
            &shell::sendto_dir(),
            std::path::Path::new(""),
            false,
        ))),
        Cli::RemoveMenu => {
            // autostart goes too: this is the uninstall path
            let a = shell::set_autostart(false);
            Some(code(shell::remove().and(a)))
        }
        Cli::InstallFfmpeg => Some(install_ffmpeg_headless()),
        Cli::Uninstall {
            quiet: true,
            keep_data,
        } => Some(install::uninstall_quiet(*keep_data)),
        Cli::Install { dir, portable } => {
            let req = install::Request {
                dir: dir.clone(),
                portable: *portable,
            };
            Some(i32::from(install::run_install(&req, |_| {}).is_err()))
        }
        _ => None,
    }
}

/// `--install-ffmpeg`: a working ffmpeg is left alone; otherwise download it and print
/// `P <percent>` lines on stdout for the installer window.
fn install_ffmpeg_headless() -> i32 {
    use std::io::Write;
    let say = |p: f32| {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "P {p:.1}");
        let _ = out.flush();
    };
    if ffmpeg::is_working(&ffmpeg::exe_path()) {
        say(100.0);
        return 0;
    }
    i32::from(ffmpeg::install_pinned(&mut |p| say(p)).is_err())
}

struct SetupState {
    mode: install::Mode,
    exe: Mutex<Option<PathBuf>>,
}

#[tauri::command]
fn install_defaults(state: State<SetupState>) -> install::Defaults {
    install::defaults(state.mode)
}

/// Folder picker for the installer; the result already ends in `Tessera`.
#[tauri::command]
async fn pick_install_dir(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app
        .dialog()
        .file()
        .blocking_pick_folder()?
        .into_path()
        .ok()?;
    let dir = install::normalize_dir(&picked.to_string_lossy()).ok()?;
    Some(dir.to_string_lossy().into_owned())
}

#[tauri::command]
fn install_start(app: AppHandle, req: install::Request) -> Result<String, String> {
    let dir = install::normalize_dir(&req.dir)?;
    std::thread::spawn(move || {
        match install::run_install(&req, |p| {
            let _ = app.emit("install-progress", p);
        }) {
            Ok(done) => {
                *app.state::<SetupState>().exe.lock().unwrap() = Some(done.exe);
                let _ = app.emit("install-done", done.ffmpeg);
            }
            Err(message) => {
                log::write("error", &format!("install failed: {message}"));
                let _ = app.emit("install-error", message);
            }
        }
    });
    Ok(dir.to_string_lossy().into_owned())
}

#[tauri::command]
fn uninstall_start(app: AppHandle, keep_data: bool) -> Result<(), String> {
    std::thread::spawn(move || {
        match install::run_uninstall(keep_data, |p| {
            let _ = app.emit("install-progress", p);
        }) {
            Ok(()) => {}
            Err(message) => {
                let _ = app.emit("install-error", message);
            }
        }
    });
    Ok(())
}

/// "Open Tessera" on the last screen: start the installed copy and quit the installer.
#[tauri::command]
fn install_open(app: AppHandle, state: State<SetupState>) {
    if let Some(exe) = state.exe.lock().unwrap().clone() {
        let _ = std::process::Command::new(exe).spawn();
    }
    app.exit(0);
}

#[tauri::command]
fn install_close(app: AppHandle) {
    app.exit(0);
}

/// The installer window: nothing else of the app (tray, watchers, queue) starts.
fn run_setup(mode: install::Mode) {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            install_defaults,
            pick_install_dir,
            install_start,
            install_open,
            install_close,
            uninstall_start
        ])
        .setup(move |app| {
            app.manage(SetupState {
                mode,
                exe: Mutex::new(None),
            });
            if let Some(cfg) = app.config().app.windows.iter().find(|c| c.label == "setup") {
                let mut w = tauri::WebviewWindowBuilder::from_config(app.handle(), cfg)?;
                let tr = i18n::system_lang() == i18n::Lang::Tr;
                w = w.title(match (mode, tr) {
                    (install::Mode::Uninstall, true) => "Tessera Kaldır",
                    (install::Mode::Uninstall, false) => "Uninstall Tessera",
                    (_, true) => "Tessera Kurulum",
                    (_, false) => "Tessera Setup",
                });
                w.build()?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start the installer");
}

pub fn run() {
    log::install_panic_hook();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = cli::parse(&args, None);
    // subcommands never reach the Tauri runtime
    let cmd = match cmd {
        Cli::Cmd(c) => std::process::exit(headless::run(c)),
        other => other,
    };
    if let Some(code) = run_headless(&cmd) {
        std::process::exit(code);
    }
    if let Some(mode) = install::setup_mode(&cmd) {
        return run_setup(mode);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let args = argv.get(1..).unwrap_or_default();
            match cli::parse(args, Some(Path::new(&cwd))) {
                Cli::Convert { target, files } => start_convert(app, target, files),
                Cli::Preset { id, files } => start_preset(app, &id, files),
                Cli::Merge { kind, files } => start_merge(app, kind, files),
                Cli::Folder { action, dir } => start_folder(app, action, dir),
                Cli::Drop { files } => handle_drop(app, files, false),
                Cli::Custom { files } => start_custom(app, files),
                Cli::Normal => show_window(app),
                _ => {}
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_settings,
            get_catalog,
            set_settings,
            install_context_menu,
            remove_context_menu,
            cancel_job,
            ffmpeg_status,
            retry_ffmpeg_download,
            open_folder,
            panel_preview,
            list_presets,
            save_preset,
            delete_preset,
            set_preset_hidden,
            get_history,
            clear_history,
            undo_job,
            check_update,
            open_url,
            get_watch,
            set_watch,
            drop_files,
            get_drop_files,
            get_drop_options,
            run_drop,
            close_drop_window,
            get_licenses,
            app_info,
            open_log
        ])
        .on_window_event(|w, e| {
            if let WindowEvent::CloseRequested { api, .. } = e {
                api.prevent_close();
                let _ = w.hide();
            }
        })
        .setup(move |app| {
            let path = settings::settings_path();
            let first_run = !path.exists();
            let upgraded =
                !first_run && settings::stored_version(&path) < settings::SETTINGS_VERSION;
            let s = settings::load_initial(&path);
            i18n::set_lang(s.language);
            // state first: the config windows start loading pages that invoke commands right away
            let first_run_settings = (first_run || upgraded).then(|| s.clone());
            app.manage(AppState {
                queue: Queue::new(app.handle().clone(), s.panel, s.history_enabled),
                settings: Mutex::new(s),
                download: Arc::default(),
                merges: Arc::new(Mutex::new(Collector::new(batch::QUIET_MS))),
                watcher: Mutex::new(None),
                dropped: Mutex::new(vec![]),
                drop_custom: std::sync::atomic::AtomicBool::new(false),
                customs: Arc::default(),
            });
            // windows are built only now so their pages never invoke a command before the state exists
            for cfg in app
                .config()
                .app
                .windows
                .iter()
                .filter(|c| c.label != "setup")
            {
                tauri::WebviewWindowBuilder::from_config(app.handle(), cfg)?.build()?;
            }
            if let Some(w) = app.get_webview_window("progress") {
                panel::init(&w);
            }
            if let Some(s) = first_run_settings {
                // best effort: the UI can retry through install_context_menu
                let _ = shell::install(&s);
                let _ = settings::save_to(&path, &s);
            }
            ensure_ffmpeg(app.handle());
            restart_watch(app.handle());
            if app
                .state::<AppState>()
                .settings
                .lock()
                .unwrap()
                .updates
                .check_on_start
            {
                check_update_on_start(app.handle());
            }

            let mut tray = TrayIconBuilder::with_id(TRAY_ID)
                .tooltip(app_info::APP_NAME)
                .menu(&tray_menu(app.handle(), i18n::lang())?)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = e
                    {
                        show_window(tray.app_handle());
                    }
                });
            tray = tray.icon(tray_icon(taskbar_is_light()));
            tray.build(app)?;
            watch_taskbar_theme(app.handle().clone());

            // --background and --convert stay hidden; a plain launch shows the settings window
            match &cmd {
                Cli::Convert { target, files } => {
                    start_convert(app.handle(), *target, files.clone())
                }
                Cli::Preset { id, files } => start_preset(app.handle(), id, files.clone()),
                Cli::Merge { kind, files } => start_merge(app.handle(), *kind, files.clone()),
                Cli::Folder { action, dir } => {
                    start_folder(app.handle(), action.clone(), dir.clone())
                }
                Cli::Drop { files } => handle_drop(app.handle(), files.clone(), false),
                Cli::Custom { files } => start_custom(app.handle(), files.clone()),
                Cli::Normal => show_window(app.handle()),
                _ => {}
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to start the application")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(w) = app.state::<AppState>().watcher.lock().unwrap().take() {
                    w.stop();
                }
            }
        });
}
