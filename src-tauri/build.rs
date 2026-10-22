fn main() {
    // the exe icon is embedded at link time: rebuild when the artwork changes
    println!("cargo:rerun-if-changed=icons");
    tauri_build::build()
}
