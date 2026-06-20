use crate::domain::{AppearanceTheme, TextEncoding, UiLanguage, APP_AUTHOR_URL};
use crate::error::{
    AppError, IoUserMessage, PlatformUserMessage, SqliteUserMessage, TextEncodingUserMessage,
    TextFileTooLargeUserMessage,
};

#[derive(Clone, Copy)]
pub(super) struct UiText {
    language: UiLanguage,
}

pub(super) fn ui_text(language: UiLanguage) -> UiText {
    UiText { language }
}

pub(super) fn app_error_user_message(error: &AppError, language: UiLanguage) -> String {
    match error {
        AppError::DatabaseOpen { path, .. } => match language {
            UiLanguage::Korean => format!(
                "문서 DB를 열 수 없습니다.\n경로: {}\n쓰기 권한, 파일 잠금, 디스크 용량을 확인하세요.",
                path.display()
            ),
            UiLanguage::English => format!(
                "Cannot open the document database.\nPath: {}\nCheck write permissions, file locks, and disk space.",
                path.display()
            ),
        },
        AppError::Domain(error) => match error {
            crate::domain::DomainError::CannotDeleteRoot => match language {
                UiLanguage::Korean => "루트 문서는 삭제할 수 없습니다.".to_owned(),
                UiLanguage::English => "The root document cannot be deleted.".to_owned(),
            },
            crate::domain::DomainError::CannotMoveNodeIntoDescendant { .. }
            | crate::domain::DomainError::CannotMoveNodeIntoItself { .. } => match language {
                UiLanguage::Korean => "자기 자신이나 하위 문서로 이동할 수 없습니다.".to_owned(),
                UiLanguage::English => {
                    "Cannot move a document into itself or its descendants.".to_owned()
                }
            },
            crate::domain::DomainError::CannotMoveRoot => match language {
                UiLanguage::Korean => "루트 문서는 이동할 수 없습니다.".to_owned(),
                UiLanguage::English => "The root document cannot be moved.".to_owned(),
            },
            crate::domain::DomainError::DocumentSaveConflict { .. } => match language {
                UiLanguage::Korean => {
                    "다른 곳에서 먼저 저장되었습니다. 다시 불러오거나 새 문서로 저장하세요."
                        .to_owned()
                }
                UiLanguage::English => {
                    "This document was saved elsewhere first. Reload it or save your content as a new document."
                        .to_owned()
                }
            },
            crate::domain::DomainError::DuplicateSiblingTitle { .. } => match language {
                UiLanguage::Korean => "같은 위치에 같은 이름이 있습니다.".to_owned(),
                UiLanguage::English => {
                    "A document with the same name already exists in this location.".to_owned()
                }
            },
            crate::domain::DomainError::EmptyTitle { .. }
            | crate::domain::DomainError::EmptyTitleInput => match language {
                UiLanguage::Korean => "이름을 입력하세요.".to_owned(),
                UiLanguage::English => "Enter a name.".to_owned(),
            },
            crate::domain::DomainError::NodeNotFound { .. } => match language {
                UiLanguage::Korean => {
                    "선택한 문서를 찾을 수 없습니다. 트리를 새로 고치세요.".to_owned()
                }
                UiLanguage::English => {
                    "The selected document was not found. Refresh the tree.".to_owned()
                }
            },
            crate::domain::DomainError::NodeNotDeleted { .. } => match language {
                UiLanguage::Korean => "선택한 문서가 휴지통에 없습니다.".to_owned(),
                UiLanguage::English => "The selected document is not in the trash.".to_owned(),
            },
            _ => match language {
                UiLanguage::Korean => "문서 데이터가 올바르지 않습니다.".to_owned(),
                UiLanguage::English => "The document data is not valid.".to_owned(),
            },
        },
        AppError::Io {
            user_message: IoUserMessage::ReadTextFile,
            ..
        } => match language {
            UiLanguage::Korean => {
                "텍스트 파일을 읽을 수 없습니다. 경로와 권한을 확인하세요.".to_owned()
            }
            UiLanguage::English => {
                "Cannot read the text file. Check the path and permissions.".to_owned()
            }
        },
        AppError::Io {
            user_message: IoUserMessage::WriteTextFile,
            ..
        } => match language {
            UiLanguage::Korean => {
                "텍스트 파일을 저장할 수 없습니다. 경로, 권한, 디스크 용량을 확인하세요."
                    .to_owned()
            }
            UiLanguage::English => {
                "Cannot save the text file. Check the path, permissions, and disk space.".to_owned()
            }
        },
        AppError::Io { .. } => match language {
            UiLanguage::Korean => "파일 작업에 실패했습니다. 쓰기 권한을 확인하세요.".to_owned(),
            UiLanguage::English => {
                "The file operation failed. Check write permissions.".to_owned()
            }
        },
        AppError::Platform { user_message, .. } => platform_user_message(*user_message, language),
        AppError::Sqlite {
            user_message: SqliteUserMessage::SaveDocumentContent,
            ..
        } => {
            match language {
                UiLanguage::Korean => {
                    "문서를 저장할 수 없습니다. DB 파일 권한과 디스크 용량을 확인하세요."
                        .to_owned()
                }
                UiLanguage::English => {
                    "Cannot save the document. Check DB file permissions and disk space."
                        .to_owned()
                }
            }
        }
        AppError::Sqlite { .. } => match language {
            UiLanguage::Korean => "문서 DB를 열거나 초기화할 수 없습니다.".to_owned(),
            UiLanguage::English => "Cannot open or initialize the document database.".to_owned(),
        },
        AppError::TextEncoding {
            user_message: TextEncodingUserMessage::Encode,
            encoding,
            ..
        } => match language {
            UiLanguage::Korean => format!(
                "선택한 인코딩({})으로 저장할 수 없는 문자가 있습니다.\nUTF-8 또는 UTF-16으로 내보내세요.",
                ui_text(language).text_encoding_name(*encoding)
            ),
            UiLanguage::English => format!(
                "Some characters cannot be saved with the selected encoding ({}).\nExport with UTF-8 or UTF-16.",
                ui_text(language).text_encoding_name(*encoding)
            ),
        },
        AppError::TextEncoding { encoding, .. } => match language {
            UiLanguage::Korean => format!(
                "선택한 인코딩({})으로 읽을 수 없습니다.\n다른 인코딩을 선택하세요.",
                ui_text(language).text_encoding_name(*encoding)
            ),
            UiLanguage::English => format!(
                "Cannot read this file with the selected encoding ({}).\nChoose a different encoding.",
                ui_text(language).text_encoding_name(*encoding)
            ),
        },
        AppError::TextFileTooLarge {
            user_message,
            limit_mib,
        } => {
            text_file_too_large_message(*user_message, *limit_mib, language)
        }
        AppError::User { message } => message.clone(),
    }
}

fn platform_user_message(user_message: PlatformUserMessage, language: UiLanguage) -> String {
    match user_message {
        PlatformUserMessage::DesktopUiUnsupported => match language {
            UiLanguage::Korean => "현재 플랫폼에서 데스크톱 UI를 사용할 수 없습니다.".to_owned(),
            UiLanguage::English => {
                "The desktop UI is not available on the current platform.".to_owned()
            }
        },
        PlatformUserMessage::Win32Startup => match language {
            UiLanguage::Korean => "Windows 창을 시작하지 못했습니다.".to_owned(),
            UiLanguage::English => "Could not start the Windows window.".to_owned(),
        },
        PlatformUserMessage::RichEditStartup => match language {
            UiLanguage::Korean => "Windows 본문 편집기를 시작하지 못했습니다.".to_owned(),
            UiLanguage::English => "Could not start the Windows document editor.".to_owned(),
        },
        PlatformUserMessage::Font => match language {
            UiLanguage::Korean => "글꼴을 적용할 수 없습니다. 다른 글꼴을 선택하세요.".to_owned(),
            UiLanguage::English => "Cannot apply the font. Choose a different font.".to_owned(),
        },
        PlatformUserMessage::Generic => match language {
            UiLanguage::Korean => "Windows UI 오류입니다. 다시 시도하세요.".to_owned(),
            UiLanguage::English => "Windows UI error. Try again.".to_owned(),
        },
    }
}

fn text_file_too_large_message(
    user_message: TextFileTooLargeUserMessage,
    limit_mib: usize,
    language: UiLanguage,
) -> String {
    match (language, user_message) {
        (UiLanguage::Korean, TextFileTooLargeUserMessage::Import) => {
            format!("가져올 텍스트가 너무 큽니다. {limit_mib}MiB 이하로 나누어 다시 시도하세요.")
        }
        (UiLanguage::Korean, TextFileTooLargeUserMessage::Export) => {
            format!("내보낼 텍스트가 너무 큽니다. {limit_mib}MiB 이하로 나누어 다시 시도하세요.")
        }
        (UiLanguage::Korean, TextFileTooLargeUserMessage::Generic) => {
            format!("텍스트가 너무 큽니다. {limit_mib}MiB 이하로 나누어 다시 시도하세요.")
        }
        (UiLanguage::English, TextFileTooLargeUserMessage::Import) => format!(
            "The text to import is too large. Split it into {limit_mib}MiB or smaller chunks and try again."
        ),
        (UiLanguage::English, TextFileTooLargeUserMessage::Export) => format!(
            "The text to export is too large. Split it into {limit_mib}MiB or smaller chunks and try again."
        ),
        (UiLanguage::English, TextFileTooLargeUserMessage::Generic) => format!(
            "The text is too large. Split it into {limit_mib}MiB or smaller chunks and try again."
        ),
    }
}

impl UiText {
    pub(super) fn menu_file(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "파일",
            UiLanguage::English => "File",
        }
    }

    pub(super) fn menu_edit(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "편집",
            UiLanguage::English => "Edit",
        }
    }

    pub(super) fn menu_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "문서",
            UiLanguage::English => "Document",
        }
    }

    pub(super) fn menu_view(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "보기",
            UiLanguage::English => "View",
        }
    }

    pub(super) fn menu_help(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "도움말",
            UiLanguage::English => "Help",
        }
    }

    pub(super) fn save_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "저장\tCtrl+S",
            UiLanguage::English => "Save\tCtrl+S",
        }
    }

    pub(super) fn import_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "가져오기...",
            UiLanguage::English => "Import...",
        }
    }

    pub(super) fn import_encoding(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "가져올 인코딩",
            UiLanguage::English => "Import Encoding",
        }
    }

    pub(super) fn export_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "내보내기...",
            UiLanguage::English => "Export...",
        }
    }

    pub(super) fn export_all_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "모든 문서 내보내기...",
            UiLanguage::English => "Export All Documents...",
        }
    }

    pub(super) fn export_encoding(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "내보낼 인코딩",
            UiLanguage::English => "Export Encoding",
        }
    }

    pub(super) fn close_tab(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "탭 닫기\tCtrl+W",
            UiLanguage::English => "Close Tab\tCtrl+W",
        }
    }

    pub(super) fn close_window(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "종료",
            UiLanguage::English => "Exit",
        }
    }

    pub(super) fn undo(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "실행 취소",
            UiLanguage::English => "Undo",
        }
    }

    pub(super) fn cut(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "잘라내기",
            UiLanguage::English => "Cut",
        }
    }

    pub(super) fn copy(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "복사",
            UiLanguage::English => "Copy",
        }
    }

    pub(super) fn paste(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "붙여넣기",
            UiLanguage::English => "Paste",
        }
    }

    pub(super) fn delete_selection(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "삭제",
            UiLanguage::English => "Delete",
        }
    }

    pub(super) fn select_all(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "전체 선택\tCtrl+A",
            UiLanguage::English => "Select All\tCtrl+A",
        }
    }

    pub(super) fn find_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "찾기...\tCtrl+F",
            UiLanguage::English => "Find...\tCtrl+F",
        }
    }

    pub(super) fn replace_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "바꾸기...\tCtrl+H",
            UiLanguage::English => "Replace...\tCtrl+H",
        }
    }

    pub(super) fn new_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "새 문서\tCtrl+N",
            UiLanguage::English => "New Document\tCtrl+N",
        }
    }

    pub(super) fn new_child_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "새 하위 문서\tCtrl+Enter",
            UiLanguage::English => "New Child Document\tCtrl+Enter",
        }
    }

    pub(super) fn rename(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "이름 변경\tF2",
            UiLanguage::English => "Rename\tF2",
        }
    }

    pub(super) fn move_up(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "위로\tCtrl+Up",
            UiLanguage::English => "Move Up\tCtrl+Up",
        }
    }

    pub(super) fn move_down(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "아래로\tCtrl+Down",
            UiLanguage::English => "Move Down\tCtrl+Down",
        }
    }

    pub(super) fn move_to_trash(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "휴지통으로 이동\tDelete",
            UiLanguage::English => "Move to Trash\tDelete",
        }
    }

    pub(super) fn restore(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "복원",
            UiLanguage::English => "Restore",
        }
    }

    pub(super) fn delete_permanently(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "영구 삭제",
            UiLanguage::English => "Delete Permanently",
        }
    }

    pub(super) fn document_tree(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "문서 트리",
            UiLanguage::English => "Document Tree",
        }
    }

    pub(super) fn trash(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "휴지통",
            UiLanguage::English => "Trash",
        }
    }

    pub(super) fn theme(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "테마",
            UiLanguage::English => "Theme",
        }
    }

    pub(super) fn language(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "언어",
            UiLanguage::English => "Language",
        }
    }

    pub(super) fn word_wrap(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "자동 줄바꿈",
            UiLanguage::English => "Word Wrap",
        }
    }

    pub(super) fn editor_font(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "편집기 글꼴...",
            UiLanguage::English => "Editor Font...",
        }
    }

    pub(super) fn about_menu(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "j3TreeText 정보",
            UiLanguage::English => "About j3TreeText",
        }
    }

    pub(super) fn search_cue(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "제목 또는 본문 검색",
            UiLanguage::English => "Search title or content",
        }
    }

    pub(super) fn caret_position_status(self, line: usize, column: usize) -> String {
        match self.language {
            UiLanguage::Korean => format!("줄 {line}, 열 {column}"),
            UiLanguage::English => format!("Ln {line}, Col {column}"),
        }
    }

    pub(super) fn text_encoding_name(self, encoding: TextEncoding) -> &'static str {
        match encoding {
            TextEncoding::AutoDetect => match self.language {
                UiLanguage::Korean => "자동 감지",
                UiLanguage::English => "Auto Detect",
            },
            TextEncoding::Utf8 => "UTF-8",
            TextEncoding::Utf8WithBom => "UTF-8 BOM",
            TextEncoding::Utf16LeWithBom => "UTF-16 LE BOM",
            TextEncoding::Utf16BeWithBom => "UTF-16 BE BOM",
            TextEncoding::KoreanEucKr => match self.language {
                UiLanguage::Korean => "한국어 (EUC-KR/CP949)",
                UiLanguage::English => "Korean (EUC-KR/CP949)",
            },
            TextEncoding::Windows1252 => "Windows-1252",
        }
    }

    pub(super) fn theme_name(self, theme: AppearanceTheme) -> &'static str {
        match theme {
            AppearanceTheme::Light => match self.language {
                UiLanguage::Korean => "밝게",
                UiLanguage::English => "Light",
            },
            AppearanceTheme::ClassicDark => match self.language {
                UiLanguage::Korean => "어둡게",
                UiLanguage::English => "Dark",
            },
            AppearanceTheme::SepiaTeal => match self.language {
                UiLanguage::Korean => "세피아",
                UiLanguage::English => "Sepia",
            },
            AppearanceTheme::Graphite => match self.language {
                UiLanguage::Korean => "그래파이트",
                UiLanguage::English => "Graphite",
            },
            AppearanceTheme::Forest => match self.language {
                UiLanguage::Korean => "숲",
                UiLanguage::English => "Forest",
            },
            AppearanceTheme::SteelBlue => match self.language {
                UiLanguage::Korean => "스틸 블루",
                UiLanguage::English => "Steel Blue",
            },
        }
    }

    pub(super) fn deleted_title(self, title: &str) -> String {
        match self.language {
            UiLanguage::Korean => format!("[삭제됨] {title}"),
            UiLanguage::English => format!("[Deleted] {title}"),
        }
    }

    pub(super) fn search_result_title(self, title: &str, parent_title: Option<&str>) -> String {
        let parent_title = parent_title.unwrap_or_else(|| self.no_parent());
        match self.language {
            UiLanguage::Korean => format!("{title} (부모: {parent_title})"),
            UiLanguage::English => format!("{title} (Parent: {parent_title})"),
        }
    }

    pub(super) fn no_parent(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "상위 없음",
            UiLanguage::English => "No parent",
        }
    }

    pub(super) fn save_conflict_message(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => concat!(
                "다른 곳에서 먼저 저장되었습니다.\n",
                "현재 내용을 덮어쓰지 않습니다.\n\n",
                "예: 최신 내용 다시 불러오기\n",
                "아니요: 새 문서로 저장\n",
                "취소: 계속 편집"
            ),
            UiLanguage::English => concat!(
                "This document was saved elsewhere first.\n",
                "Your current content will not overwrite it.\n\n",
                "Yes: reload the latest content\n",
                "No: save as a new document\n",
                "Cancel: keep editing"
            ),
        }
    }

    pub(super) fn conflicted_copy_title(self, title: &str) -> String {
        match self.language {
            UiLanguage::Korean => format!("{title} (복구됨)"),
            UiLanguage::English => format!("{title} (recovered)"),
        }
    }

    pub(super) fn missing_find_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "찾을 내용을 입력하세요.",
            UiLanguage::English => "Enter text to find.",
        }
    }

    pub(super) fn open_editable_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "편집 가능한 문서를 먼저 여세요.",
            UiLanguage::English => "Open an editable document first.",
        }
    }

    pub(super) fn read_only_find_replace(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "읽기 전용 문서에서는 찾기/바꾸기를 사용할 수 없습니다.",
            UiLanguage::English => "Find/replace is not available in read-only documents.",
        }
    }

    pub(super) fn no_match(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "일치 항목이 없습니다.",
            UiLanguage::English => "No matches found.",
        }
    }

    pub(super) fn replace_all_too_large(self, limit_mib: usize) -> String {
        match self.language {
            UiLanguage::Korean => format!("결과가 너무 큽니다. {limit_mib}MiB 이하로 줄이세요."),
            UiLanguage::English => {
                format!("The result is too large. Keep it under {limit_mib}MiB.")
            }
        }
    }

    pub(super) fn replace_all_overflow(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => {
                "결과가 처리 가능한 범위를 넘었습니다. 문서나 바꿀 내용을 줄이세요."
            }
            UiLanguage::English => {
                "The result is too large to process. Reduce the document or replacement text."
            }
        }
    }

    pub(super) fn replace_all_allocation_failed(self, requested: usize) -> String {
        match self.language {
            UiLanguage::Korean => format!("메모리가 부족합니다. 예상 결과: {requested}바이트"),
            UiLanguage::English => {
                format!("Not enough memory. Estimated result: {requested} bytes")
            }
        }
    }

    pub(super) fn replace_all_count(self, count: usize) -> String {
        match self.language {
            UiLanguage::Korean => format!("{count}개 변경됨"),
            UiLanguage::English => format!("{count} replacements made"),
        }
    }

    pub(super) fn unsaved_changes(self, title: Option<&str>) -> String {
        match (self.language, title) {
            (UiLanguage::Korean, Some(title)) => {
                format!("\"{title}\"에 저장하지 않은 변경이 있습니다.\n저장할까요?")
            }
            (UiLanguage::Korean, None) => "저장하지 않은 변경이 있습니다.\n저장할까요?".to_owned(),
            (UiLanguage::English, Some(title)) => {
                format!("\"{title}\" has unsaved changes.\nSave them?")
            }
            (UiLanguage::English, None) => "There are unsaved changes.\nSave them?".to_owned(),
        }
    }

    pub(super) fn discard_unsavable_changes(self, title: Option<&str>) -> String {
        match (self.language, title) {
            (UiLanguage::Korean, Some(title)) => {
                format!("\"{title}\"은 저장할 수 없습니다.\n변경을 버리고 계속할까요?")
            }
            (UiLanguage::Korean, None) => {
                "현재 저장할 수 없습니다.\n변경을 버리고 계속할까요?".to_owned()
            }
            (UiLanguage::English, Some(title)) => {
                format!("\"{title}\" cannot be saved.\nDiscard changes and continue?")
            }
            (UiLanguage::English, None) => {
                "The current changes cannot be saved.\nDiscard them and continue?".to_owned()
            }
        }
    }

    pub(super) fn window_trash_suffix(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "휴지통",
            UiLanguage::English => "Trash",
        }
    }

    pub(super) fn window_search_suffix(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "검색",
            UiLanguage::English => "Search",
        }
    }

    pub(super) fn about_title(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "j3TreeText 정보",
            UiLanguage::English => "About j3TreeText",
        }
    }

    pub(super) fn about_message(self, version: &str) -> String {
        match self.language {
            UiLanguage::Korean => format!("j3TreeText {version}"),
            UiLanguage::English => format!("j3TreeText {version}"),
        }
    }

    pub(super) fn about_hyperlink_content(self, version: &str) -> String {
        format!(
            "{}\n\n<A HREF=\"{APP_AUTHOR_URL}\">{APP_AUTHOR_URL}</A>",
            self.about_message(version)
        )
    }

    pub(super) fn open_import_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "가져올 문서를 먼저 여세요.",
            UiLanguage::English => "Open a document before importing.",
        }
    }

    pub(super) fn open_export_document(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "내보낼 문서를 먼저 여세요.",
            UiLanguage::English => "Open a document before exporting.",
        }
    }

    pub(super) fn imported_text_nul(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => {
                "가져온 텍스트에 지원하지 않는 NUL 문자가 있습니다. 해당 문자를 제거하세요."
            }
            UiLanguage::English => {
                "The imported text contains unsupported NUL characters. Remove them first."
            }
        }
    }

    pub(super) fn imported_text_too_large(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "가져온 텍스트가 너무 큽니다. 파일을 나누어 가져오세요.",
            UiLanguage::English => "The imported text is too large. Split the file and try again.",
        }
    }

    pub(super) fn export_text_too_large(self, limit_mib: usize) -> String {
        match self.language {
            UiLanguage::Korean => {
                format!("내보낼 텍스트가 너무 큽니다. {limit_mib}MiB 이하로 나누세요.")
            }
            UiLanguage::English => {
                format!("The text to export is too large. Split it into {limit_mib}MiB or smaller chunks.")
            }
        }
    }

    pub(super) fn export_all_complete_title(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "모든 문서 내보내기",
            UiLanguage::English => "Export All Documents",
        }
    }

    pub(super) fn export_all_complete_message(self, count: usize, path: &str) -> String {
        match self.language {
            UiLanguage::Korean => format!("{count}개 문서를 내보냈습니다.\n경로: {path}"),
            UiLanguage::English => {
                format!("Exported {count} documents.\nPath: {path}")
            }
        }
    }

    pub(super) fn font_fallback(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "선택한 글꼴을 사용할 수 없어 기본 글꼴을 적용했습니다.",
            UiLanguage::English => {
                "The selected font is unavailable. The default font was applied."
            }
        }
    }

    pub(super) fn active_tree_only(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "휴지통 보기에서는 사용할 수 없습니다.",
            UiLanguage::English => "This command is not available in Trash view.",
        }
    }

    pub(super) fn search_not_allowed(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "검색 중에는 사용할 수 없습니다. 검색어를 지우세요.",
            UiLanguage::English => {
                "This command is not available while searching. Clear the search text."
            }
        }
    }

    pub(super) fn trash_only(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "휴지통 보기에서만 사용할 수 있습니다.",
            UiLanguage::English => "This command is only available in Trash view.",
        }
    }

    pub(super) fn confirm_delete(self, title: &str) -> String {
        match self.language {
            UiLanguage::Korean => {
                format!("\"{title}\"을 휴지통으로 이동할까요?\n하위 문서도 함께 이동합니다.")
            }
            UiLanguage::English => {
                format!("Move \"{title}\" to the trash?\nChild documents will also be moved.")
            }
        }
    }

    pub(super) fn confirm_restore(self, title: &str) -> String {
        match self.language {
            UiLanguage::Korean => {
                format!("\"{title}\"을 복원할까요?\n원래 위치가 없으면 루트 아래로 이동합니다.")
            }
            UiLanguage::English => {
                format!("Restore \"{title}\"?\nIf the original location is missing, it will be moved under the root.")
            }
        }
    }

    pub(super) fn confirm_permanent_delete(self, title: &str) -> String {
        match self.language {
            UiLanguage::Korean => {
                format!("\"{title}\"을 영구 삭제할까요?\n하위 문서도 함께 삭제됩니다.")
            }
            UiLanguage::English => {
                format!("Permanently delete \"{title}\"?\nChild documents will also be deleted.")
            }
        }
    }

    pub(super) fn file_dialog_import_title(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "텍스트 가져오기",
            UiLanguage::English => "Import Text",
        }
    }

    pub(super) fn file_dialog_export_title(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "텍스트 내보내기",
            UiLanguage::English => "Export Text",
        }
    }

    pub(super) fn file_dialog_export_folder_title(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "모든 문서 내보낼 폴더 선택",
            UiLanguage::English => "Choose Export Folder",
        }
    }

    pub(super) fn file_filter_text(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "텍스트 파일 (*.txt)",
            UiLanguage::English => "Text Files (*.txt)",
        }
    }

    pub(super) fn file_filter_all(self) -> &'static str {
        match self.language {
            UiLanguage::Korean => "모든 파일 (*.*)",
            UiLanguage::English => "All Files (*.*)",
        }
    }

    pub(super) fn file_dialog_error(self, code: u32) -> String {
        match self.language {
            UiLanguage::Korean => format!("파일 대화상자를 열 수 없습니다.\n오류 코드: {code}"),
            UiLanguage::English => format!("Cannot open the file dialog.\nError code: {code}"),
        }
    }

    pub(super) fn font_dialog_error(self, code: u32) -> String {
        match self.language {
            UiLanguage::Korean => format!("글꼴 대화상자를 열 수 없습니다.\n오류 코드: {code}"),
            UiLanguage::English => format!("Cannot open the font dialog.\nError code: {code}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_hyperlink_content_uses_task_dialog_anchor() {
        assert_eq!(
            ui_text(UiLanguage::English).about_hyperlink_content("1.2.3"),
            "j3TreeText 1.2.3\n\n<A HREF=\"https://github.com/edgarp9\">https://github.com/edgarp9</A>"
        );
        assert_eq!(
            ui_text(UiLanguage::Korean).about_hyperlink_content("1.2.3"),
            "j3TreeText 1.2.3\n\n<A HREF=\"https://github.com/edgarp9\">https://github.com/edgarp9</A>"
        );
    }
}
