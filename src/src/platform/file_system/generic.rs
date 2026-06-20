use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::Path;

pub(super) fn prepare_replacement_file(
    temp_path: &Path,
    path: &Path,
    file: File,
) -> io::Result<File> {
    super::copy_existing_permissions(temp_path, path)?;
    Ok(file)
}

pub(super) fn create_replacement_file(temp_path: &Path, _path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
}

pub(super) fn replace_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    fs::rename(temp_path, path)?;
    sync_parent_directory(path)
}

fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()
}
