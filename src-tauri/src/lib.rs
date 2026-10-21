pub mod app_info;
pub mod cli;
pub mod convert;
pub mod ffmpeg;
pub mod panel;
pub mod queue;
pub mod settings;
pub mod shell;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_notification::NotificationExt;

use cli::Cli;
use ffmpeg::{Download, FfmpegStatus};
use queue::Queue;
use settings::Settings;

struct AppState {
    settings: Mutex<Settings>,
    queue: Queue,
    download: Arc<Download>,
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
    notify(
        app,
        "ffmpeg indiriliyor",
        "Video ve MP3/OGG dönüştürme için ücretsiz, açık kaynaklı ffmpeg aracı bir kez indirilecek (yaklaşık 170 MB).",
    );
    let app = app.clone();
    std::thread::spawn(move || {
        let result = ffmpeg::install_with_retries(&download);
        if result.is_ok() {
            set_ffmpeg_ready(&app, true);
            notify(
                &app,
                "ffmpeg hazır",
                "Video ve ses dönüştürme kullanıma hazır.",
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
fn set_settings(settings: Settings, state: State<AppState>) -> Result<Settings, String> {
    settings.validate()?;
    let mut cur = state.settings.lock().unwrap();
    let mut new = settings.normalized();
    new.ffmpeg_ready = cur.ffmpeg_ready;
    shell::set_autostart(new.autostart)
        .map_err(|e| format!("Otomatik başlatma ayarlanamadı: {e}"))?;
    shell::install(&new).map_err(|e| format!("Sağ tık menüsü kaydedilemedi: {e}"))?;
    settings::save_to(&settings::settings_path(), &new)
        .map_err(|e| format!("Ayarlar kaydedilemedi: {e}"))?;
    *state.queue.panel.lock().unwrap() = new.panel;
    *cur = new.clone();
    Ok(new)
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
    shell::install(&s).map_err(|e| format!("Sağ tık menüsü kaydedilemedi: {e}"))
}

#[tauri::command]
fn remove_context_menu() -> Result<(), String> {
    shell::remove().map_err(|e| format!("Sağ tık menüsü kaldırılamadı: {e}"))
}

#[tauri::command]
fn cancel_job(id: u64, state: State<AppState>) {
    state.queue.cancel(id);
}

#[tauri::command]
fn ffmpeg_status(state: State<AppState>) -> FfmpegStatus {
    let ready = state.settings.lock().unwrap().ffmpeg_ready;
    state.download.status(ready)
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
        p.corner = serde_json::from_value(c.into()).map_err(|_| "Geçersiz köşe".to_string())?;
    }
    if let Some(s) = seconds {
        if !(2..=30).contains(&s) {
            return Err("Geçersiz süre".into());
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
        return Err("Geçersiz yol".into());
    }
    let p = Path::new(&path);
    if !p.exists() {
        return Err("Yol bulunamadı".into());
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
        notify(
            app,
            "Dönüştürme başarısız",
            &format!(
                "{} biçimi ayarlardan kapatılmış",
                target.ext().to_uppercase()
            ),
        );
        return;
    }
    state
        .queue
        .enqueue(app, target, files, &s.to_options(), s.ffmpeg_ready);
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
            panel_preview
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
            let s = settings::load_from(&path);
            if first_run {
                // best effort: the UI can retry through install_context_menu
                let _ = shell::install(&s);
                let _ = settings::save_to(&path, &s);
            }
            if let Some(w) = app.get_webview_window("progress") {
                panel::init(&w);
            }
            app.manage(AppState {
                queue: Queue::new(app.handle().clone(), s.panel),
                settings: Mutex::new(s),
                download: Arc::default(),
            });
            ensure_ffmpeg(app.handle());

            let open = MenuItem::with_id(app, "open", "Ayarlar", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?;
            let mut tray = TrayIconBuilder::new()
                .tooltip(app_info::APP_NAME)
                .menu(&Menu::with_items(app, &[&open, &quit])?)
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
                Cli::Normal => show_window(app.handle()),
                _ => {}
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("uygulama başlatılamadı");
}
