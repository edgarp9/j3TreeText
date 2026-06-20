#[cfg(windows)]
use crate::app::App;
#[cfg(windows)]
use crate::error::AppError;

pub(crate) mod command_contract;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(not(any(windows, target_os = "linux")))]
mod unsupported;

#[cfg(windows)]
pub fn run_message_loop(app: App) -> Result<(), AppError> {
    super::win32::run_message_loop(app)
}

#[cfg(windows)]
pub fn show_error_message(title: &str, message: &str) {
    super::win32::show_error_message(title, message);
}

#[cfg(target_os = "linux")]
pub use linux::{run_message_loop, show_error_message};

#[cfg(not(any(windows, target_os = "linux")))]
pub use unsupported::{run_message_loop, show_error_message};
