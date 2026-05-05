fn main() {
    if let Err(err) = app_lib::run() {
        eprintln!("Cretar IA failed to start: {err}");
        std::process::exit(1);
    }
}
