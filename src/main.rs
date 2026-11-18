use std::process::ExitCode;

use presentationpoint::{application, logging};

fn main() -> ExitCode {
    if let Err(error) = logging::initialize() {
        eprintln!("Presentationpoint could not initialize logging: {error}");
        return ExitCode::FAILURE;
    }

    match application::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = %error, "Presentationpoint stopped with an error");
            eprintln!("Presentationpoint: {error}");
            ExitCode::FAILURE
        }
    }
}
