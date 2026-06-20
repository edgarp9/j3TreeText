use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::domain::{DomainError, TextEncoding};

const INTERNAL_CONSISTENCY_USER_MESSAGE: &str =
    "문서 데이터가 올바르지 않습니다. 트리를 새로 고치거나 문서를 다시 여세요.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoUserMessage {
    Generic,
    ReadTextFile,
    WriteTextFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformUserMessage {
    Generic,
    DesktopUiUnsupported,
    Win32Startup,
    RichEditStartup,
    Font,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqliteUserMessage {
    Generic,
    SaveDocumentContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncodingUserMessage {
    Decode,
    Encode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextFileTooLargeUserMessage {
    Generic,
    Import,
    Export,
}

impl TextFileTooLargeUserMessage {
    fn action(self) -> &'static str {
        match self {
            Self::Generic => "process",
            Self::Import => "import",
            Self::Export => "export",
        }
    }
}

#[derive(Debug)]
pub enum AppError {
    DatabaseOpen {
        path: PathBuf,
        source: rusqlite::Error,
    },
    Domain(DomainError),
    Io {
        action: &'static str,
        user_message: IoUserMessage,
        source: io::Error,
    },
    Platform {
        action: &'static str,
        user_message: PlatformUserMessage,
        message: String,
    },
    Sqlite {
        action: &'static str,
        user_message: SqliteUserMessage,
        source: rusqlite::Error,
    },
    TextEncoding {
        action: &'static str,
        user_message: TextEncodingUserMessage,
        encoding: TextEncoding,
        detail: String,
    },
    TextFileTooLarge {
        user_message: TextFileTooLargeUserMessage,
        limit_mib: usize,
    },
    User {
        message: String,
    },
}

impl AppError {
    pub fn database_open(path: PathBuf, source: rusqlite::Error) -> Self {
        Self::DatabaseOpen { path, source }
    }

    pub fn io(action: &'static str, source: io::Error) -> Self {
        Self::io_with_user_message(action, IoUserMessage::Generic, source)
    }

    pub fn internal_consistency(_action: &'static str, _message: impl Into<String>) -> Self {
        Self::user(INTERNAL_CONSISTENCY_USER_MESSAGE)
    }

    pub fn io_with_user_message(
        action: &'static str,
        user_message: IoUserMessage,
        source: io::Error,
    ) -> Self {
        Self::Io {
            action,
            user_message,
            source,
        }
    }

    pub fn platform(action: &'static str, message: impl Into<String>) -> Self {
        Self::platform_with_user_message(action, PlatformUserMessage::Generic, message)
    }

    pub fn platform_with_user_message(
        action: &'static str,
        user_message: PlatformUserMessage,
        message: impl Into<String>,
    ) -> Self {
        Self::Platform {
            action,
            user_message,
            message: message.into(),
        }
    }

    pub fn sqlite(action: &'static str, source: rusqlite::Error) -> Self {
        Self::sqlite_with_user_message(action, SqliteUserMessage::Generic, source)
    }

    pub fn sqlite_with_user_message(
        action: &'static str,
        user_message: SqliteUserMessage,
        source: rusqlite::Error,
    ) -> Self {
        Self::Sqlite {
            action,
            user_message,
            source,
        }
    }

    pub fn text_encoding(
        action: &'static str,
        encoding: TextEncoding,
        detail: impl Into<String>,
    ) -> Self {
        Self::text_encoding_with_user_message(
            action,
            TextEncodingUserMessage::Decode,
            encoding,
            detail,
        )
    }

    pub fn text_encoding_with_user_message(
        action: &'static str,
        user_message: TextEncodingUserMessage,
        encoding: TextEncoding,
        detail: impl Into<String>,
    ) -> Self {
        Self::TextEncoding {
            action,
            user_message,
            encoding,
            detail: detail.into(),
        }
    }

    pub fn text_file_too_large(
        user_message: TextFileTooLargeUserMessage,
        limit_mib: usize,
    ) -> Self {
        Self::TextFileTooLarge {
            user_message,
            limit_mib,
        }
    }

    pub fn user(message: impl Into<String>) -> Self {
        Self::User {
            message: message.into(),
        }
    }

    pub fn user_message(&self) -> String {
        match self {
            Self::DatabaseOpen { path, .. } => format!(
                "문서 DB를 열 수 없습니다.\n경로: {}\n쓰기 권한, 파일 잠금, 디스크 용량을 확인하세요.",
                path.display()
            ),
            Self::Domain(error) => match error {
                DomainError::CannotDeleteRoot => "루트 문서는 삭제할 수 없습니다.".to_owned(),
                DomainError::CannotMoveNodeIntoDescendant { .. }
                | DomainError::CannotMoveNodeIntoItself { .. } => {
                    "자기 자신이나 하위 문서로 이동할 수 없습니다.".to_owned()
                }
                DomainError::CannotMoveRoot => "루트 문서는 이동할 수 없습니다.".to_owned(),
                DomainError::DocumentSaveConflict { .. } => {
                    "다른 곳에서 먼저 저장되었습니다. 다시 불러오거나 새 문서로 저장하세요."
                        .to_owned()
                }
                DomainError::DuplicateSiblingTitle { .. } => {
                    "같은 위치에 같은 이름이 있습니다.".to_owned()
                }
                DomainError::EmptyTitle { .. } | DomainError::EmptyTitleInput => {
                    "이름을 입력하세요.".to_owned()
                }
                DomainError::NodeNotFound { .. } => {
                    "선택한 문서를 찾을 수 없습니다. 트리를 새로 고치세요.".to_owned()
                }
                DomainError::NodeNotDeleted { .. } => {
                    "선택한 문서가 휴지통에 없습니다.".to_owned()
                }
                _ => "문서 데이터가 올바르지 않습니다.".to_owned(),
            },
            Self::Io {
                user_message: IoUserMessage::ReadTextFile,
                ..
            } => {
                "텍스트 파일을 읽을 수 없습니다. 경로와 권한을 확인하세요.".to_owned()
            }
            Self::Io {
                user_message: IoUserMessage::WriteTextFile,
                ..
            } => {
                "텍스트 파일을 저장할 수 없습니다. 경로, 권한, 디스크 용량을 확인하세요.".to_owned()
            }
            Self::Io { .. } => {
                "파일 작업에 실패했습니다. 쓰기 권한을 확인하세요.".to_owned()
            }
            Self::Platform { user_message, .. } => platform_user_message(*user_message),
            Self::Sqlite {
                user_message: SqliteUserMessage::SaveDocumentContent,
                ..
            } => {
                "문서를 저장할 수 없습니다. DB 파일 권한과 디스크 용량을 확인하세요.".to_owned()
            }
            Self::Sqlite { .. } => "문서 DB를 열거나 초기화할 수 없습니다.".to_owned(),
            Self::TextEncoding {
                user_message: TextEncodingUserMessage::Encode,
                encoding,
                ..
            } => format!(
                "선택한 인코딩({})으로 저장할 수 없는 문자가 있습니다.\nUTF-8 또는 UTF-16으로 내보내세요.",
                encoding.display_name()
            ),
            Self::TextEncoding { encoding, .. } => format!(
                "선택한 인코딩({})으로 읽을 수 없습니다.\n다른 인코딩을 선택하세요.",
                encoding.display_name()
            ),
            Self::TextFileTooLarge {
                user_message,
                limit_mib,
            } => match *user_message {
                TextFileTooLargeUserMessage::Import => format!(
                    "가져올 텍스트가 너무 큽니다. {limit_mib}MiB 이하로 나누어 다시 시도하세요."
                ),
                TextFileTooLargeUserMessage::Export => format!(
                    "내보낼 텍스트가 너무 큽니다. {limit_mib}MiB 이하로 나누어 다시 시도하세요."
                ),
                TextFileTooLargeUserMessage::Generic => format!(
                    "텍스트가 너무 큽니다. {limit_mib}MiB 이하로 나누어 다시 시도하세요."
                ),
            },
            Self::User { message } => message.clone(),
        }
    }
}

fn platform_user_message(user_message: PlatformUserMessage) -> String {
    match user_message {
        PlatformUserMessage::DesktopUiUnsupported => {
            "현재 플랫폼에서 데스크톱 UI를 사용할 수 없습니다.".to_owned()
        }
        PlatformUserMessage::Win32Startup => "Windows 창을 시작하지 못했습니다.".to_owned(),
        PlatformUserMessage::RichEditStartup => {
            "Windows 본문 편집기를 시작하지 못했습니다.".to_owned()
        }
        PlatformUserMessage::Font => {
            "글꼴을 적용할 수 없습니다. 다른 글꼴을 선택하세요.".to_owned()
        }
        PlatformUserMessage::Generic => "데스크톱 UI 오류입니다. 다시 시도하세요.".to_owned(),
    }
}

impl From<DomainError> for AppError {
    fn from(error: DomainError) -> Self {
        Self::Domain(error)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DatabaseOpen { path, source } => {
                write!(
                    formatter,
                    "open SQLite database at {}: {source}",
                    path.display()
                )
            }
            Self::Domain(error) => write!(formatter, "domain error: {error}"),
            Self::Io { action, source, .. } => write!(formatter, "{action}: {source}"),
            Self::Platform {
                action, message, ..
            } => write!(formatter, "{action}: {message}"),
            Self::Sqlite { action, source, .. } => write!(formatter, "{action}: {source}"),
            Self::TextEncoding {
                action,
                encoding,
                detail,
                ..
            } => write!(
                formatter,
                "{action} as {}: {detail}",
                encoding.display_name()
            ),
            Self::TextFileTooLarge {
                user_message,
                limit_mib,
            } => {
                write!(
                    formatter,
                    "{} text exceeds {limit_mib}MiB",
                    user_message.action()
                )
            }
            Self::User { message } => write!(formatter, "user action: {message}"),
        }
    }
}

impl Error for AppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DatabaseOpen { source, .. } => Some(source),
            Self::Domain(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::Sqlite { source, .. } => Some(source),
            Self::Platform { .. }
            | Self::TextEncoding { .. }
            | Self::TextFileTooLarge { .. }
            | Self::User { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_runtime_errors_do_not_claim_startup_failure() {
        let error = AppError::platform("handle editor change", "window state was not attached");

        assert_eq!(
            error.user_message(),
            "데스크톱 UI 오류입니다. 다시 시도하세요."
        );
    }

    #[test]
    fn internal_consistency_errors_use_document_data_user_message() {
        let error = AppError::internal_consistency(
            "recalculate sibling sort order",
            "sibling order contains duplicate nodes",
        );

        assert_eq!(error.user_message(), INTERNAL_CONSISTENCY_USER_MESSAGE);
        assert!(error.source().is_none());
    }

    #[test]
    fn platform_font_errors_have_font_specific_user_message() {
        let error = AppError::platform_with_user_message(
            "choose editor font",
            PlatformUserMessage::Font,
            "window state was not attached",
        );

        assert_eq!(
            error.user_message(),
            "글꼴을 적용할 수 없습니다. 다른 글꼴을 선택하세요."
        );
    }

    #[test]
    fn platform_startup_errors_keep_startup_user_message() {
        let error = AppError::platform_with_user_message(
            "create main window",
            PlatformUserMessage::Win32Startup,
            "Win32 error code 1",
        );

        assert_eq!(error.user_message(), "Windows 창을 시작하지 못했습니다.");
    }

    #[test]
    fn rich_edit_startup_errors_have_editor_specific_user_message() {
        let error = AppError::platform_with_user_message(
            "load Rich Edit library",
            PlatformUserMessage::RichEditStartup,
            "Win32 error code 126",
        );

        assert_eq!(
            error.user_message(),
            "Windows 본문 편집기를 시작하지 못했습니다."
        );
    }
}
