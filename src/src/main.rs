#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use j3treetext::app::App;
use j3treetext::domain::{
    APP_DISPLAY_NAME, APP_ICON_PNG_FILE_NAME, APP_ICON_SVG_FILE_NAME, APP_LINUX_APPLICATION_ID,
};
use j3treetext::error::AppError;
use j3treetext::infra::desktop_entry::{self, DesktopEntryMetadata};
use j3treetext::platform::gui;

fn main() {
    if let Err(error) = run() {
        let user_message = error.user_message();
        eprintln!("{user_message}");
        eprintln!("detail: {error}");
        gui::show_error_message("j3TreeText", &user_message);
        std::process::exit(1);
    }
}

fn run() -> Result<(), AppError> {
    match cli_command_from_args(std::env::args_os().skip(1))? {
        CliCommand::Run { database_path } => run_gui(database_path.as_deref()),
        CliCommand::Install => install_desktop_entry(),
        CliCommand::Uninstall => uninstall_desktop_entry(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    Run { database_path: Option<PathBuf> },
    Install,
    Uninstall,
}

fn cli_command_from_args<I, S>(args: I) -> Result<CliCommand, AppError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut args = args
        .into_iter()
        .map(Into::into)
        .filter(|arg| !arg.as_os_str().is_empty());
    let Some(command) = args.next() else {
        return Ok(CliCommand::Run {
            database_path: None,
        });
    };
    if command == "--install" {
        ensure_no_extra_args(args)?;
        return Ok(CliCommand::Install);
    }
    if command == "--uninstall" {
        ensure_no_extra_args(args)?;
        return Ok(CliCommand::Uninstall);
    }
    if command
        .to_str()
        .is_some_and(|value| value.starts_with("--"))
    {
        return Err(AppError::user(format!(
            "알 수 없는 옵션입니다: {}",
            command.to_string_lossy()
        )));
    }
    ensure_no_extra_run_args(args)?;
    Ok(CliCommand::Run {
        database_path: Some(PathBuf::from(command)),
    })
}

fn ensure_no_extra_run_args(mut args: impl Iterator<Item = OsString>) -> Result<(), AppError> {
    if let Some(extra) = args.next() {
        return Err(AppError::user(format!(
            "설정 파일 인자는 하나만 지정할 수 있습니다. 알 수 없는 인자: {}",
            extra.to_string_lossy()
        )));
    }
    Ok(())
}

fn ensure_no_extra_args(mut args: impl Iterator<Item = OsString>) -> Result<(), AppError> {
    if args.next().is_some() {
        return Err(AppError::user(
            "--install/--uninstall 명령에는 추가 인자를 지정할 수 없습니다.",
        ));
    }
    Ok(())
}

fn install_desktop_entry() -> Result<(), AppError> {
    let summary = desktop_entry::install(desktop_entry_metadata())
        .map_err(|error| AppError::platform("install desktop entry", error.to_string()))?;
    if summary.desktop_entry_changed || summary.icon_changed {
        println!(
            "desktop entry를 설치했습니다: {}",
            summary.desktop_entry_path.display()
        );
    } else {
        println!(
            "desktop entry가 이미 최신 상태입니다: {}",
            summary.desktop_entry_path.display()
        );
    }
    Ok(())
}

fn uninstall_desktop_entry() -> Result<(), AppError> {
    let summary = desktop_entry::uninstall(desktop_entry_metadata())
        .map_err(|error| AppError::platform("uninstall desktop entry", error.to_string()))?;
    if summary.desktop_entry_removed || summary.icon_removed {
        println!(
            "desktop entry를 제거했습니다: {}",
            summary.desktop_entry_path.display()
        );
    } else {
        println!(
            "desktop entry가 이미 제거된 상태입니다: {}",
            summary.desktop_entry_path.display()
        );
    }
    Ok(())
}

fn desktop_entry_metadata() -> DesktopEntryMetadata {
    DesktopEntryMetadata {
        application_id: APP_LINUX_APPLICATION_ID,
        display_name: APP_DISPLAY_NAME,
        comment: APP_DISPLAY_NAME,
        categories: "Utility;",
        icon_svg_file_name: APP_ICON_SVG_FILE_NAME,
        icon_png_file_name: APP_ICON_PNG_FILE_NAME,
        legacy_application_ids: &["io.github.j3treetext", "j3TreeText", "j3treetext"],
    }
}

fn run_gui(database_path: Option<&Path>) -> Result<(), AppError> {
    let app = App::start_with_database_path(database_path)?;
    eprintln!(
        "j3TreeText opened {} with {} active nodes.",
        app.database_path().display(),
        app.node_count()
    );
    gui::run_message_loop(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_args_run_gui_with_default_database() -> Result<(), AppError> {
        assert_eq!(
            cli_command_from_args(Vec::<OsString>::new())?,
            CliCommand::Run {
                database_path: None
            }
        );
        Ok(())
    }

    #[test]
    fn single_non_option_arg_runs_gui_with_database_path() -> Result<(), AppError> {
        assert_eq!(
            cli_command_from_args(["notes.db"])?,
            CliCommand::Run {
                database_path: Some(PathBuf::from("notes.db"))
            }
        );
        Ok(())
    }

    #[test]
    fn install_and_uninstall_commands_are_parsed() -> Result<(), AppError> {
        assert_eq!(cli_command_from_args(["--install"])?, CliCommand::Install);
        assert_eq!(
            cli_command_from_args(["--uninstall"])?,
            CliCommand::Uninstall
        );
        Ok(())
    }

    #[test]
    fn install_rejects_extra_args() {
        assert!(cli_command_from_args(["--install", "extra"]).is_err());
    }

    #[test]
    fn unknown_long_option_is_rejected() {
        assert!(cli_command_from_args(["--missing"]).is_err());
    }
}
