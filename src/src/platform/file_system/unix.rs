use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::unix::fs::OpenOptionsExt;
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
        .mode(0o600)
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

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn replacement_file_is_private() -> Result<(), Box<dyn Error>> {
        let path = unique_test_file("replacement-private");

        let result = (|| -> Result<(), Box<dyn Error>> {
            let file = create_replacement_file(&path, Path::new("unused"))?;
            drop(file);

            let mode = fs::metadata(&path)?.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            Ok(())
        })();
        let cleanup = remove_test_file(&path);

        result?;
        cleanup?;
        Ok(())
    }

    fn unique_test_file(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!(
            "j3treetext-{name}-{}-{nanos}.file",
            std::process::id()
        ))
    }

    fn remove_test_file(path: &Path) -> Result<(), Box<dyn Error>> {
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}
