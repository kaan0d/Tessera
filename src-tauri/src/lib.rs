pub mod app_info;
pub mod batch;
pub mod cli;
pub mod convert;
pub mod ffmpeg;
pub mod history;
pub mod i18n;
pub mod panel;
pub mod preset;
pub mod queue;
pub mod settings;
pub mod shell;

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
use ffmpeg::{Download, FfmpegStatus};
use history::Kind;
use i18n::{fmt, t, Lang};
use queue::{Queue, Spec};
use settings::Settings;

const TRAY_ID: &str = "main";

struct AppState {
    settings: Mutex<Settings>,
    queue: Queue,
    download: Arc<Download>,
    merges: Arc<Mutex<Collector>>,
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
struct CatalogSource {
    ext: &'static str,
    kind: &'static str,
}

#[derive(Serialize)]
struct CatalogTarget {
    ext: &'static str,
    kind: &'static str,
    needs_ffmpeg: bool,
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

/// Reveals a file in Explorer (or opens a folder).
#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    if path.contains('"') || path.starts_with(r"\\") || path.starts_with("//") {
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
    std::process::Command::new("explorer")
        .raw_arg(arg)
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
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
            let s = settings::load_from(&settings::settings_path());
            Some(code(shell::install(&s)))
        }
        Cli::RemoveMenu => {
            // autostart goes too: this is the uninstall path
            let a = shell::set_autostart(false);
            Some(code(shell::remove().and(a)))
        }
        _ => None,
    }
}

pub fn run() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = cli::parse(&args, None);
    if let Some(code) = run_headless(&cmd) {
        std::process::exit(code);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
            let args = argv.get(1..).unwrap_or_default();
            match cli::parse(args, Some(Path::new(&cwd))) {
                Cli::Convert { target, files } => start_convert(app, target, files),
                Cli::Preset { id, files } => start_preset(app, &id, files),
                Cli::Merge { kind, files } => start_merge(app, kind, files),
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
            undo_job
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
            let s = settings::load_from(&path);
            i18n::set_lang(s.language);
            if first_run || upgraded {
                // best effort: the UI can retry through install_context_menu
                let _ = shell::install(&s);
                let _ = settings::save_to(&path, &s);
            }
            if let Some(w) = app.get_webview_window("progress") {
                panel::init(&w);
            }
            app.manage(AppState {
                queue: Queue::new(app.handle().clone(), s.panel, s.history_enabled),
                settings: Mutex::new(s),
                download: Arc::default(),
                merges: Arc::new(Mutex::new(Collector::new(batch::QUIET_MS))),
            });
            ensure_ffmpeg(app.handle());

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
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            // --background and --convert stay hidden; a plain launch shows the settings window
            match &cmd {
                Cli::Convert { target, files } => {
                    start_convert(app.handle(), *target, files.clone())
                }
                Cli::Preset { id, files } => start_preset(app.handle(), id, files.clone()),
                Cli::Merge { kind, files } => start_merge(app.handle(), *kind, files.clone()),
                Cli::Normal => show_window(app.handle()),
                _ => {}
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to start the application");
}
