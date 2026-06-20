use std::error::Error;
use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopEntryMetadata {
    pub application_id: &'static str,
    pub display_name: &'static str,
    pub comment: &'static str,
    pub categories: &'static str,
    pub icon_svg_file_name: &'static str,
    pub icon_png_file_name: &'static str,
    pub legacy_application_ids: &'static [&'static str],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntryInstallSummary {
    pub desktop_entry_path: PathBuf,
    pub icon_path: PathBuf,
    pub desktop_entry_changed: bool,
    pub icon_changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntryUninstallSummary {
    pub desktop_entry_path: PathBuf,
    pub icon_path: PathBuf,
    pub desktop_entry_removed: bool,
    pub icon_removed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntryError {
    message: String,
}

impl DesktopEntryError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for DesktopEntryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for DesktopEntryError {}

pub fn install(
    metadata: DesktopEntryMetadata,
) -> Result<DesktopEntryInstallSummary, DesktopEntryError> {
    imp::install(metadata)
}

pub fn uninstall(
    metadata: DesktopEntryMetadata,
) -> Result<DesktopEntryUninstallSummary, DesktopEntryError> {
    imp::uninstall(metadata)
}

#[cfg(target_os = "linux")]
mod imp {
    use std::ffi::OsString;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use super::{
        DesktopEntryError, DesktopEntryInstallSummary, DesktopEntryMetadata,
        DesktopEntryUninstallSummary,
    };

    const HICOLOR_PNG_ICON_SIZE_DIR: &str = "256x256";

    pub fn install(
        metadata: DesktopEntryMetadata,
    ) -> Result<DesktopEntryInstallSummary, DesktopEntryError> {
        let executable_path = current_executable_path()?;
        let icon_source =
            find_icon_source_path(metadata.icon_svg_file_name, metadata.icon_png_file_name)?;
        let data_home = xdg_data_home()?;
        let mut summary = install_into(metadata, &executable_path, &icon_source, &data_home)?;
        let legacy_removed = remove_legacy_entries(&data_home, metadata)?;
        summary.desktop_entry_changed |= legacy_removed.desktop_entry_removed;
        summary.icon_changed |= legacy_removed.icon_removed;
        refresh_desktop_caches(&data_home);
        Ok(summary)
    }

    pub fn uninstall(
        metadata: DesktopEntryMetadata,
    ) -> Result<DesktopEntryUninstallSummary, DesktopEntryError> {
        let data_home = xdg_data_home()?;
        let summary = uninstall_from_data_home(metadata, &data_home)?;
        refresh_desktop_caches(&data_home);
        Ok(summary)
    }

    fn uninstall_from_data_home(
        metadata: DesktopEntryMetadata,
        data_home: &Path,
    ) -> Result<DesktopEntryUninstallSummary, DesktopEntryError> {
        let paths = DesktopEntryPaths::new(data_home, metadata.application_id);
        let removed = remove_application_files(data_home, metadata.application_id)?;
        let mut desktop_entry_removed = removed.desktop_entry_removed;
        let mut icon_removed = removed.icon_removed;
        if let Some(alias_id) = lowercase_alias_id(metadata.application_id) {
            let removed = remove_application_files(data_home, &alias_id)?;
            desktop_entry_removed |= removed.desktop_entry_removed;
            icon_removed |= removed.icon_removed;
        }
        let legacy_removed = remove_legacy_entries(data_home, metadata)?;
        Ok(DesktopEntryUninstallSummary {
            desktop_entry_path: paths.desktop_entry_path,
            icon_path: paths.icon_svg_path,
            desktop_entry_removed: desktop_entry_removed || legacy_removed.desktop_entry_removed,
            icon_removed: icon_removed || legacy_removed.icon_removed,
        })
    }

    fn current_executable_path() -> Result<PathBuf, DesktopEntryError> {
        std::env::current_exe().map_err(|source| {
            DesktopEntryError::new(format!("실행 파일 경로를 확인할 수 없습니다: {source}"))
        })
    }

    fn xdg_data_home() -> Result<PathBuf, DesktopEntryError> {
        if let Some(data_home) = non_empty_env_path("XDG_DATA_HOME") {
            if data_home.is_absolute() {
                return Ok(data_home);
            }
            return Err(DesktopEntryError::new(
                "XDG_DATA_HOME은 절대 경로여야 합니다.",
            ));
        }
        let Some(home) = non_empty_env_path("HOME") else {
            return Err(DesktopEntryError::new(
                "HOME 경로를 확인할 수 없어 desktop entry를 설치할 수 없습니다.",
            ));
        };
        Ok(home.join(".local/share"))
    }

    fn non_empty_env_path(key: &str) -> Option<PathBuf> {
        std::env::var_os(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    }

    fn find_icon_source_path(
        svg_file_name: &str,
        png_file_name: &str,
    ) -> Result<IconSource, DesktopEntryError> {
        find_icon_source_path_in_dirs(svg_file_name, png_file_name, &icon_search_dirs())
    }

    fn icon_search_dirs() -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        if let Ok(exe_path) = std::env::current_exe() {
            if let Some(exe_dir) = exe_path.parent() {
                candidates.push(exe_dir.to_path_buf());
            }
        }
        if let Ok(current_dir) = std::env::current_dir() {
            candidates.push(current_dir);
        }
        candidates
    }

    fn find_icon_source_path_in_dirs(
        svg_file_name: &str,
        png_file_name: &str,
        candidates: &[PathBuf],
    ) -> Result<IconSource, DesktopEntryError> {
        for candidate in candidates {
            let path = candidate.join(svg_file_name);
            if path.is_file() {
                return Ok(IconSource {
                    path,
                    format: IconFormat::Svg,
                });
            }
        }
        for candidate in candidates {
            let path = candidate.join(png_file_name);
            if path.is_file() {
                return Ok(IconSource {
                    path,
                    format: IconFormat::Png,
                });
            }
        }
        Err(DesktopEntryError::new(format!(
            "아이콘 파일을 찾을 수 없습니다: {svg_file_name} 또는 {png_file_name}"
        )))
    }

    fn install_into(
        metadata: DesktopEntryMetadata,
        executable_path: &Path,
        icon_source: &IconSource,
        data_home: &Path,
    ) -> Result<DesktopEntryInstallSummary, DesktopEntryError> {
        let paths = DesktopEntryPaths::new(data_home, metadata.application_id);
        fs::create_dir_all(&paths.applications_dir).map_err(|source| {
            io_error(
                "desktop entry 디렉터리를 만들 수 없습니다",
                &paths.applications_dir,
                source,
            )
        })?;
        let icon_dir = icon_source.format.icon_dir(&paths);
        fs::create_dir_all(icon_dir)
            .map_err(|source| io_error("아이콘 디렉터리를 만들 수 없습니다", icon_dir, source))?;

        let desktop_entry = desktop_entry_content(metadata, executable_path);
        let desktop_entry_changed =
            write_text_if_changed(&paths.desktop_entry_path, &desktop_entry)?;
        let mut desktop_entry_changed = desktop_entry_changed;
        let alias_id = lowercase_alias_id(metadata.application_id);
        if let Some(alias_id) = alias_id.as_deref() {
            let alias_paths = DesktopEntryPaths::new(data_home, alias_id);
            let alias_entry = alias_desktop_entry_content(metadata, alias_id, executable_path);
            desktop_entry_changed |=
                write_text_if_changed(&alias_paths.desktop_entry_path, &alias_entry)?;
        }

        let icon_path = icon_source.format.icon_path(&paths);
        let installed_icon_path = icon_path.to_path_buf();
        let mut icon_changed = copy_file_if_changed(&icon_source.path, icon_path)?;
        if let Some(alias_id) = alias_id.as_deref() {
            let alias_paths = DesktopEntryPaths::new(data_home, alias_id);
            icon_changed |= copy_file_if_changed(
                &icon_source.path,
                icon_source.format.icon_path(&alias_paths),
            )?;
        }
        if icon_source.format == IconFormat::Svg {
            icon_changed |= remove_file_if_exists(&paths.icon_png_path)?;
            if let Some(alias_id) = alias_id.as_deref() {
                let alias_paths = DesktopEntryPaths::new(data_home, alias_id);
                icon_changed |= remove_file_if_exists(&alias_paths.icon_png_path)?;
            }
        } else {
            icon_changed |= remove_file_if_exists(&paths.icon_svg_path)?;
            if let Some(alias_id) = alias_id.as_deref() {
                let alias_paths = DesktopEntryPaths::new(data_home, alias_id);
                icon_changed |= remove_file_if_exists(&alias_paths.icon_svg_path)?;
            }
        }

        Ok(DesktopEntryInstallSummary {
            desktop_entry_path: paths.desktop_entry_path,
            icon_path: installed_icon_path,
            desktop_entry_changed,
            icon_changed,
        })
    }

    fn desktop_entry_content(metadata: DesktopEntryMetadata, executable_path: &Path) -> String {
        desktop_entry_content_for_id(metadata, metadata.application_id, executable_path, false)
    }

    fn alias_desktop_entry_content(
        metadata: DesktopEntryMetadata,
        alias_id: &str,
        executable_path: &Path,
    ) -> String {
        desktop_entry_content_for_id(metadata, alias_id, executable_path, true)
    }

    fn desktop_entry_content_for_id(
        metadata: DesktopEntryMetadata,
        desktop_id: &str,
        executable_path: &Path,
        no_display: bool,
    ) -> String {
        let no_display_line = if no_display { "NoDisplay=true\n" } else { "" };
        format!(
            "# Managed by {} --install\n\
             [Desktop Entry]\n\
             Type=Application\n\
             Name={}\n\
             Comment={}\n\
             Exec={}\n\
             Icon={desktop_id}\n\
             Terminal=false\n\
             Categories={}\n\
             StartupNotify=true\n\
             StartupWMClass={desktop_id}\n\
             {no_display_line}",
            metadata.display_name,
            metadata.display_name,
            metadata.comment,
            desktop_exec_path(executable_path),
            metadata.categories,
        )
    }

    fn desktop_exec_path(path: &Path) -> String {
        let value = path.to_string_lossy();
        if value
            .chars()
            .all(|ch| !ch.is_whitespace() && !matches!(ch, '"' | '\\' | '$' | '`'))
        {
            return value.into_owned();
        }

        let mut escaped = String::from("\"");
        for ch in value.chars() {
            match ch {
                '"' | '\\' | '$' | '`' => {
                    escaped.push('\\');
                    escaped.push(ch);
                }
                _ => escaped.push(ch),
            }
        }
        escaped.push('"');
        escaped
    }

    fn write_text_if_changed(path: &Path, content: &str) -> Result<bool, DesktopEntryError> {
        if fs::read_to_string(path).is_ok_and(|existing| existing == content) {
            return Ok(false);
        }
        fs::write(path, content)
            .map_err(|source| io_error("desktop entry를 쓸 수 없습니다", path, source))?;
        Ok(true)
    }

    fn copy_file_if_changed(
        source_path: &Path,
        destination_path: &Path,
    ) -> Result<bool, DesktopEntryError> {
        let source = fs::read(source_path)
            .map_err(|source| io_error("아이콘 파일을 읽을 수 없습니다", source_path, source))?;
        if fs::read(destination_path).is_ok_and(|existing| existing == source) {
            return Ok(false);
        }
        fs::write(destination_path, source).map_err(|source| {
            io_error("아이콘 파일을 설치할 수 없습니다", destination_path, source)
        })?;
        Ok(true)
    }

    fn remove_file_if_exists(path: &Path) -> Result<bool, DesktopEntryError> {
        match fs::remove_file(path) {
            Ok(()) => Ok(true),
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(io_error("파일을 제거할 수 없습니다", path, source)),
        }
    }

    fn remove_legacy_entries(
        data_home: &Path,
        metadata: DesktopEntryMetadata,
    ) -> Result<DesktopEntryUninstallSummary, DesktopEntryError> {
        let mut desktop_entry_removed = false;
        let mut icon_removed = false;
        for legacy_id in metadata
            .legacy_application_ids
            .iter()
            .copied()
            .filter(|legacy_id| *legacy_id != metadata.application_id)
        {
            let removed = remove_application_files(data_home, legacy_id)?;
            desktop_entry_removed |= removed.desktop_entry_removed;
            icon_removed |= removed.icon_removed;
            if let Some(alias_id) = lowercase_alias_id(legacy_id) {
                let removed = remove_application_files(data_home, &alias_id)?;
                desktop_entry_removed |= removed.desktop_entry_removed;
                icon_removed |= removed.icon_removed;
            }
        }
        let current_paths = DesktopEntryPaths::new(data_home, metadata.application_id);
        Ok(DesktopEntryUninstallSummary {
            desktop_entry_path: current_paths.desktop_entry_path,
            icon_path: current_paths.icon_svg_path,
            desktop_entry_removed,
            icon_removed,
        })
    }

    fn remove_application_files(
        data_home: &Path,
        application_id: &str,
    ) -> Result<RemovedApplicationFiles, DesktopEntryError> {
        let paths = DesktopEntryPaths::new(data_home, application_id);
        Ok(RemovedApplicationFiles {
            desktop_entry_removed: remove_file_if_exists(&paths.desktop_entry_path)?,
            icon_removed: remove_file_if_exists(&paths.icon_svg_path)?
                | remove_file_if_exists(&paths.icon_png_path)?,
        })
    }

    fn lowercase_alias_id(application_id: &str) -> Option<String> {
        let alias_id = application_id.to_ascii_lowercase();
        (alias_id != application_id).then_some(alias_id)
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct RemovedApplicationFiles {
        desktop_entry_removed: bool,
        icon_removed: bool,
    }

    fn refresh_desktop_caches(data_home: &Path) {
        let applications_dir = data_home.join("applications");
        let hicolor_dir = data_home.join("icons/hicolor");
        run_cache_command(
            "update-desktop-database",
            &[applications_dir.into_os_string()],
        );
        run_cache_command(
            "gtk-update-icon-cache",
            &[
                OsString::from("-f"),
                OsString::from("-t"),
                hicolor_dir.into_os_string(),
            ],
        );
        if !run_cache_command("kbuildsycoca6", &[OsString::from("--noincremental")]) {
            run_cache_command("kbuildsycoca5", &[OsString::from("--noincremental")]);
        }
    }

    fn run_cache_command(program: &str, args: &[OsString]) -> bool {
        Command::new(program)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn io_error(action: &str, path: &Path, source: io::Error) -> DesktopEntryError {
        DesktopEntryError::new(format!("{action}: {}: {source}", path.display()))
    }

    #[derive(Debug, Clone)]
    struct DesktopEntryPaths {
        applications_dir: PathBuf,
        scalable_icon_dir: PathBuf,
        png_icon_dir: PathBuf,
        desktop_entry_path: PathBuf,
        icon_svg_path: PathBuf,
        icon_png_path: PathBuf,
    }

    impl DesktopEntryPaths {
        fn new(data_home: &Path, application_id: &str) -> Self {
            let applications_dir = data_home.join("applications");
            let scalable_icon_dir = data_home.join("icons/hicolor/scalable/apps");
            let png_icon_dir = data_home
                .join("icons/hicolor")
                .join(HICOLOR_PNG_ICON_SIZE_DIR)
                .join("apps");
            Self {
                desktop_entry_path: applications_dir.join(format!("{application_id}.desktop")),
                icon_svg_path: scalable_icon_dir.join(format!("{application_id}.svg")),
                icon_png_path: png_icon_dir.join(format!("{application_id}.png")),
                applications_dir,
                scalable_icon_dir,
                png_icon_dir,
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct IconSource {
        path: PathBuf,
        format: IconFormat,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum IconFormat {
        Svg,
        Png,
    }

    impl IconFormat {
        fn icon_dir(self, paths: &DesktopEntryPaths) -> &Path {
            match self {
                Self::Svg => &paths.scalable_icon_dir,
                Self::Png => &paths.png_icon_dir,
            }
        }

        fn icon_path(self, paths: &DesktopEntryPaths) -> &Path {
            match self {
                Self::Svg => &paths.icon_svg_path,
                Self::Png => &paths.icon_png_path,
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::error::Error;
        use std::time::{SystemTime, UNIX_EPOCH};

        use super::*;

        const TEST_APP_ID: &str = "io.github.edgarp9.j3TreeText";
        const TEST_APP_ID_LOWER: &str = "io.github.edgarp9.j3treetext";

        #[test]
        fn icon_source_prefers_svg_over_png() -> Result<(), Box<dyn Error>> {
            let test_dir = unique_test_dir("icon-source-prefers-svg");
            let source_dir = test_dir.join("source");
            fs::create_dir_all(&source_dir)?;
            fs::write(source_dir.join("icon.svg"), "<svg />")?;
            fs::write(source_dir.join("icon.png"), b"png")?;

            let source =
                find_icon_source_path_in_dirs("icon.svg", "icon.png", &[source_dir.to_path_buf()])?;

            assert_eq!(source.path, source_dir.join("icon.svg"));
            assert_eq!(source.format, IconFormat::Svg);
            remove_test_dir(&test_dir)?;
            Ok(())
        }

        #[test]
        fn install_svg_writes_alias_and_removes_stale_png() -> Result<(), Box<dyn Error>> {
            let test_dir = unique_test_dir("install-svg");
            let data_home = test_dir.join("data");
            let source_dir = test_dir.join("source");
            fs::create_dir_all(&source_dir)?;
            let source_path = source_dir.join("icon.svg");
            fs::write(&source_path, "<svg />")?;

            let paths = DesktopEntryPaths::new(&data_home, TEST_APP_ID);
            let alias_paths = DesktopEntryPaths::new(&data_home, TEST_APP_ID_LOWER);
            fs::create_dir_all(&paths.png_icon_dir)?;
            fs::write(&paths.icon_png_path, b"old png")?;
            fs::write(&alias_paths.icon_png_path, b"old alias png")?;

            let summary = install_into(
                metadata(),
                Path::new("/opt/j3TreeText/j3TreeText"),
                &IconSource {
                    path: source_path,
                    format: IconFormat::Svg,
                },
                &data_home,
            )?;

            assert!(summary.desktop_entry_changed);
            assert!(summary.icon_changed);
            let desktop_entry = fs::read_to_string(&paths.desktop_entry_path)?;
            assert!(desktop_entry.contains("Name=j3TreeText\n"));
            assert!(desktop_entry.contains("Comment=j3TreeText\n"));
            assert!(desktop_entry.contains("Icon=io.github.edgarp9.j3TreeText\n"));
            assert!(desktop_entry.contains("StartupWMClass=io.github.edgarp9.j3TreeText\n"));
            assert!(!desktop_entry.contains("NoDisplay=true"));

            let alias_entry = fs::read_to_string(&alias_paths.desktop_entry_path)?;
            assert!(alias_entry.contains("Icon=io.github.edgarp9.j3treetext\n"));
            assert!(alias_entry.contains("StartupWMClass=io.github.edgarp9.j3treetext\n"));
            assert!(alias_entry.contains("NoDisplay=true\n"));
            assert_eq!(fs::read_to_string(&paths.icon_svg_path)?, "<svg />");
            assert_eq!(fs::read_to_string(&alias_paths.icon_svg_path)?, "<svg />");
            assert!(!paths.icon_png_path.exists());
            assert!(!alias_paths.icon_png_path.exists());

            remove_test_dir(&test_dir)?;
            Ok(())
        }

        #[test]
        fn install_png_fallback_removes_stale_svg() -> Result<(), Box<dyn Error>> {
            let test_dir = unique_test_dir("install-png");
            let data_home = test_dir.join("data");
            let source_dir = test_dir.join("source");
            fs::create_dir_all(&source_dir)?;
            let source_path = source_dir.join("icon.png");
            fs::write(&source_path, b"png")?;

            let paths = DesktopEntryPaths::new(&data_home, TEST_APP_ID);
            let alias_paths = DesktopEntryPaths::new(&data_home, TEST_APP_ID_LOWER);
            fs::create_dir_all(&paths.scalable_icon_dir)?;
            fs::write(&paths.icon_svg_path, "<svg />")?;
            fs::write(&alias_paths.icon_svg_path, "<svg />")?;

            let summary = install_into(
                metadata(),
                Path::new("/opt/j3TreeText/j3TreeText"),
                &IconSource {
                    path: source_path,
                    format: IconFormat::Png,
                },
                &data_home,
            )?;

            assert!(summary.icon_changed);
            assert_eq!(fs::read(&paths.icon_png_path)?, b"png");
            assert_eq!(fs::read(&alias_paths.icon_png_path)?, b"png");
            assert!(!paths.icon_svg_path.exists());
            assert!(!alias_paths.icon_svg_path.exists());

            remove_test_dir(&test_dir)?;
            Ok(())
        }

        #[test]
        fn repeated_install_with_same_content_reports_no_changes() -> Result<(), Box<dyn Error>> {
            let test_dir = unique_test_dir("install-idempotent");
            let data_home = test_dir.join("data");
            let source_dir = test_dir.join("source");
            fs::create_dir_all(&source_dir)?;
            let source_path = source_dir.join("icon.svg");
            fs::write(&source_path, "<svg />")?;
            let icon_source = IconSource {
                path: source_path,
                format: IconFormat::Svg,
            };

            install_into(
                metadata(),
                Path::new("/opt/j3TreeText/j3TreeText"),
                &icon_source,
                &data_home,
            )?;
            let second = install_into(
                metadata(),
                Path::new("/opt/j3TreeText/j3TreeText"),
                &icon_source,
                &data_home,
            )?;

            assert!(!second.desktop_entry_changed);
            assert!(!second.icon_changed);

            remove_test_dir(&test_dir)?;
            Ok(())
        }

        #[test]
        fn reinstall_after_executable_moves_updates_desktop_entry() -> Result<(), Box<dyn Error>> {
            let test_dir = unique_test_dir("install-moved-executable");
            let data_home = test_dir.join("data");
            let source_dir = test_dir.join("source");
            fs::create_dir_all(&source_dir)?;
            let source_path = source_dir.join("icon.svg");
            fs::write(&source_path, "<svg />")?;
            let icon_source = IconSource {
                path: source_path,
                format: IconFormat::Svg,
            };

            install_into(
                metadata(),
                Path::new("/opt/old/j3TreeText"),
                &icon_source,
                &data_home,
            )?;
            let second = install_into(
                metadata(),
                Path::new("/opt/new/j3TreeText"),
                &icon_source,
                &data_home,
            )?;

            assert!(second.desktop_entry_changed);
            assert!(!second.icon_changed);
            assert!(fs::read_to_string(second.desktop_entry_path)?
                .contains("Exec=/opt/new/j3TreeText\n"));

            remove_test_dir(&test_dir)?;
            Ok(())
        }

        #[test]
        fn uninstall_removes_main_alias_and_legacy_files() -> Result<(), Box<dyn Error>> {
            let test_dir = unique_test_dir("uninstall");
            let data_home = test_dir.join("data");
            for application_id in [
                TEST_APP_ID,
                TEST_APP_ID_LOWER,
                "io.github.j3treetext",
                "j3TreeText",
                "j3treetext",
            ] {
                create_installed_files(&data_home, application_id)?;
            }

            let summary = uninstall_from_data_home(metadata(), &data_home)?;

            assert!(summary.desktop_entry_removed);
            assert!(summary.icon_removed);
            for application_id in [
                TEST_APP_ID,
                TEST_APP_ID_LOWER,
                "io.github.j3treetext",
                "j3TreeText",
                "j3treetext",
            ] {
                let paths = DesktopEntryPaths::new(&data_home, application_id);
                assert!(!paths.desktop_entry_path.exists());
                assert!(!paths.icon_svg_path.exists());
                assert!(!paths.icon_png_path.exists());
            }

            remove_test_dir(&test_dir)?;
            Ok(())
        }

        #[test]
        fn uninstall_succeeds_when_files_are_already_absent() -> Result<(), Box<dyn Error>> {
            let test_dir = unique_test_dir("uninstall-absent");
            let data_home = test_dir.join("data");

            let summary = uninstall_from_data_home(metadata(), &data_home)?;

            assert!(!summary.desktop_entry_removed);
            assert!(!summary.icon_removed);
            remove_test_dir(&test_dir)?;
            Ok(())
        }

        fn metadata() -> DesktopEntryMetadata {
            DesktopEntryMetadata {
                application_id: TEST_APP_ID,
                display_name: "j3TreeText",
                comment: "j3TreeText",
                categories: "Utility;",
                icon_svg_file_name: "icon.svg",
                icon_png_file_name: "icon.png",
                legacy_application_ids: &["io.github.j3treetext", "j3TreeText", "j3treetext"],
            }
        }

        fn create_installed_files(
            data_home: &Path,
            application_id: &str,
        ) -> Result<(), Box<dyn Error>> {
            let paths = DesktopEntryPaths::new(data_home, application_id);
            fs::create_dir_all(&paths.applications_dir)?;
            fs::create_dir_all(&paths.scalable_icon_dir)?;
            fs::create_dir_all(&paths.png_icon_dir)?;
            fs::write(paths.desktop_entry_path, "[Desktop Entry]\n")?;
            fs::write(paths.icon_svg_path, "<svg />")?;
            fs::write(paths.icon_png_path, b"png")?;
            Ok(())
        }

        fn unique_test_dir(name: &str) -> PathBuf {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0);
            std::env::temp_dir().join(format!(
                "j3treetext-desktop-entry-{name}-{}-{timestamp}",
                std::process::id()
            ))
        }

        fn remove_test_dir(path: &Path) -> Result<(), Box<dyn Error>> {
            match fs::remove_dir_all(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(Box::new(error)),
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::{
        DesktopEntryError, DesktopEntryInstallSummary, DesktopEntryMetadata,
        DesktopEntryUninstallSummary,
    };

    pub fn install(
        _metadata: DesktopEntryMetadata,
    ) -> Result<DesktopEntryInstallSummary, DesktopEntryError> {
        Err(DesktopEntryError::new(
            "이 플랫폼에서는 desktop entry 설치를 지원하지 않습니다.",
        ))
    }

    pub fn uninstall(
        _metadata: DesktopEntryMetadata,
    ) -> Result<DesktopEntryUninstallSummary, DesktopEntryError> {
        Err(DesktopEntryError::new(
            "이 플랫폼에서는 desktop entry 제거를 지원하지 않습니다.",
        ))
    }
}
