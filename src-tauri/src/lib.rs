pub mod app_info;
pub mod convert;

pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("uygulama başlatılamadı");
}
