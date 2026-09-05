mod window;

#[cfg(not(test))]
use std::process::ExitCode;

#[cfg(not(test))]
fn main() -> ExitCode {
    match lynx_launcher_host::run(std::env::args_os(), |options| {
        window::run(options)?;
        Ok(())
    }) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lynx-launcher-rs: {error}");
            ExitCode::FAILURE
        }
    }
}
