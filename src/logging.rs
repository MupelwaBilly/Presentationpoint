use tracing_subscriber::EnvFilter;

use crate::AppError;

pub fn initialize() -> Result<(), AppError> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init()
        .map_err(|error| AppError::CommandLine(format!("logging setup failed: {error}")))
}
