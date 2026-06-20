use std::io;
use std::path::Path;

#[cfg(not(windows))]
use std::fs;
use std::fs::File;

#[cfg(windows)]
#[path = "windows.rs"]
mod imp;

#[cfg(all(unix, not(windows)))]
#[path = "unix.rs"]
mod imp;

#[cfg(not(any(windows, unix)))]
#[path = "generic.rs"]
mod imp;

pub(crate) fn prepare_replacement_file(
    temp_path: &Path,
    path: &Path,
    file: File,
) -> io::Result<File> {
    imp::prepare_replacement_file(temp_path, path, file)
}

pub(crate) fn create_replacement_file(temp_path: &Path, path: &Path) -> io::Result<File> {
    imp::create_replacement_file(temp_path, path)
}

pub(crate) fn replace_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    imp::replace_file(temp_path, path)
}

#[cfg(not(windows))]
fn copy_existing_permissions(temp_path: &Path, path: &Path) -> io::Result<()> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            fs::set_permissions(temp_path, metadata.permissions())
        }
        Ok(_) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(source),
    }
}

#[cfg(all(test, unix, not(windows)))]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_TEST_FILE_ID: AtomicU64 = AtomicU64::new(1);

    fn unique_test_path(name: &str) -> PathBuf {
        let id = NEXT_TEST_FILE_ID.fetch_add(1, Ordering::Relaxed);
        std::env::current_dir()
            .expect("current directory should be available")
            .join("target")
            .join(format!(
                "j3treetext-file-system-{name}-{}-{id}",
                std::process::id()
            ))
    }

    fn remove_file_if_exists(path: &Path) -> io::Result<()> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    #[test]
    fn unix_replacement_preserves_existing_file_permissions() -> io::Result<()> {
        let path = unique_test_path("target");
        let temp_path = path.with_file_name(format!(
            ".{}-replacement",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("target")
        ));
        remove_file_if_exists(&temp_path)?;
        remove_file_if_exists(&path)?;

        let result = (|| -> io::Result<()> {
            fs::write(&path, "old")?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640))?;

            let file = create_replacement_file(&temp_path, &path)?;
            let mut file = prepare_replacement_file(&temp_path, &path, file)?;
            file.write_all(b"new")?;
            file.sync_all()?;
            drop(file);
            replace_file(&temp_path, &path)?;

            assert_eq!(fs::read_to_string(&path)?, "new");
            assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o640);
            Ok(())
        })();
        let cleanup_temp = remove_file_if_exists(&temp_path);
        let cleanup_path = remove_file_if_exists(&path);

        result?;
        cleanup_temp?;
        cleanup_path
    }
}
