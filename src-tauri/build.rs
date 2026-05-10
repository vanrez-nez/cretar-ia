fn main() {
    if std::env::vars()
        .any(|(key, _)| key.starts_with("DEP_TAURI_CORE") && key.ends_with("PERMISSION_FILES_PATH"))
    {
        tauri_build::build()
    }
}
