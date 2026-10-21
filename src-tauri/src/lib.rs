pub mod app_info;

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("uygulama başlatılamadı");
}
