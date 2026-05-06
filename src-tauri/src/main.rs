fn main() {
    if let Err(err) = app_lib::run() {
        exit_with_startup_error(err);
    }
}

fn exit_with_startup_error(err: anyhow::Error) -> ! {
    eprintln!("Cretar IA failed to start: {err}");
    std::process::exit(1);
}
