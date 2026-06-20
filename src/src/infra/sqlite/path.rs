use std::path::{Path, PathBuf};

use crate::error::AppError;

pub fn database_path_for_current_exe() -> Result<PathBuf, AppError> {
    let exe_path =
        std::env::current_exe().map_err(|source| AppError::io("locate executable", source))?;
    database_path_for_executable(&exe_path)
}

pub fn database_path_for_executable(exe_path: &Path) -> Result<PathBuf, AppError> {
    if exe_path.file_name().is_none() {
        return Err(AppError::platform(
            "calculate database path",
            "executable path does not include a file name",
        ));
    }

    let mut db_path = exe_path.to_path_buf();
    db_path.set_extension("db");
    Ok(db_path)
}
