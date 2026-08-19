use std::process::ExitCode;

fn main() -> ExitCode {
    match lynx_launcher_host::run(std::env::args_os()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lynx-launcher-rs: {error}");
            ExitCode::FAILURE
        }
    }
}
