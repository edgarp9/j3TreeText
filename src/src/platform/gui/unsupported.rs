use crate::app::App;
use crate::error::{AppError, PlatformUserMessage};

pub fn run_message_loop(_app: App) -> Result<(), AppError> {
    Err(AppError::platform_with_user_message(
        "start desktop UI",
        PlatformUserMessage::DesktopUiUnsupported,
        "this platform does not have a j3TreeText desktop UI backend",
    ))
}

pub fn show_error_message(title: &str, message: &str) {
    eprintln!("{title}: {message}");
}
