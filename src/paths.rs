use std::path::{Path, PathBuf};

use crate::AppError;

/// Finds the project root from a stable project file or the installed entry point.
pub fn project_root() -> Result<PathBuf, AppError> {
    let executable =
        std::env::current_exe().map_err(|source| AppError::io("current executable", source))?;
    let executable_dir = executable
        .parent()
        .ok_or_else(|| AppError::CommandLine("executable has no parent directory".to_owned()))?;

    for candidate in executable.ancestors() {
        if candidate.join("Cargo.toml").is_file() {
            return Ok(candidate.to_path_buf());
        }
    }

    Ok(executable_dir.to_path_buf())
}

/// Resolves command-line paths from the project root, independent of terminal cwd.
pub fn resolve_project_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Ok(project_root()?.join(path))
}
