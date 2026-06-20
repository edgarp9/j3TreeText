use windows_sys::Win32::Foundation::{GetLastError, LPARAM, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{WM_APP, WM_USER};

use crate::domain::{AppearanceTheme, TextEncoding, UiLanguage};
use crate::error::{AppError, PlatformUserMessage};
use crate::platform::gui::command_contract::GuiCommand;

pub(super) const WINDOW_CLASS_NAME: &str = "j3TreeTextWindowClass";
pub(super) const APP_ICON_RESOURCE_ID: u16 = 1;
pub(super) const SEARCH_EDIT_CONTROL_CLASS_NAME: &str = "EDIT";
pub(super) const CARET_STATUS_CONTROL_CLASS_NAME: &str = "STATIC";
pub(super) const DOCUMENT_EDITOR_RICH_EDIT_CLASS_NAME: &str = "RICHEDIT50W";
pub(super) const RICH_EDIT_MODULE_NAME: &str = "Msftedit.dll";
pub(super) const SPLITTER_WIDTH: i32 = 4;
pub(super) const MIN_SPLIT_WIDTH: i32 = 80;
pub(super) const MIN_EDITOR_WIDTH: i32 = 160;
pub(super) const SEARCH_BOX_HEIGHT: i32 = 24;
pub(super) const SEARCH_PANEL_PADDING: i32 = 6;
pub(super) const TAB_BAR_HEIGHT: i32 = 28;
pub(super) const CARET_STATUS_HEIGHT: i32 = 22;
pub(super) const CARET_STATUS_HORIZONTAL_PADDING: i32 = 8;
pub(super) const COMMAND_NEW_CHILD_DOCUMENT: usize = 1001;
pub(super) const COMMAND_NEW_DOCUMENT: usize = 1002;
pub(super) const COMMAND_RENAME: usize = 1003;
pub(super) const COMMAND_DELETE: usize = 1004;
pub(super) const COMMAND_SHOW_ACTIVE_TREE: usize = 1005;
pub(super) const COMMAND_SHOW_TRASH: usize = 1006;
pub(super) const COMMAND_RESTORE: usize = 1007;
pub(super) const COMMAND_DELETE_PERMANENTLY: usize = 1008;
pub(super) const COMMAND_CLOSE_TAB: usize = 1009;
pub(super) const COMMAND_EDITOR_FONT: usize = 1010;
pub(super) const COMMAND_IMPORT_TEXT: usize = 1011;
pub(super) const COMMAND_EXPORT_TEXT: usize = 1012;
pub(super) const COMMAND_EDITOR_WORD_WRAP: usize = 1013;
pub(super) const COMMAND_EDITOR_UNDO: usize = 1014;
pub(super) const COMMAND_EDITOR_CUT: usize = 1015;
pub(super) const COMMAND_EDITOR_COPY: usize = 1016;
pub(super) const COMMAND_EDITOR_PASTE: usize = 1017;
pub(super) const COMMAND_EDITOR_DELETE_SELECTION: usize = 1018;
pub(super) const COMMAND_EDITOR_SELECT_ALL: usize = 1019;
pub(super) const COMMAND_IMPORT_ENCODING_AUTO: usize = 1020;
pub(super) const COMMAND_IMPORT_ENCODING_UTF8: usize = 1021;
pub(super) const COMMAND_IMPORT_ENCODING_UTF8_BOM: usize = 1022;
pub(super) const COMMAND_IMPORT_ENCODING_UTF16_LE_BOM: usize = 1023;
pub(super) const COMMAND_IMPORT_ENCODING_UTF16_BE_BOM: usize = 1024;
pub(super) const COMMAND_IMPORT_ENCODING_KOREAN_EUC_KR: usize = 1025;
pub(super) const COMMAND_IMPORT_ENCODING_WINDOWS_1252: usize = 1026;
pub(super) const COMMAND_EXPORT_ENCODING_UTF8: usize = 1030;
pub(super) const COMMAND_EXPORT_ENCODING_UTF8_BOM: usize = 1031;
pub(super) const COMMAND_EXPORT_ENCODING_UTF16_LE_BOM: usize = 1032;
pub(super) const COMMAND_EXPORT_ENCODING_UTF16_BE_BOM: usize = 1033;
pub(super) const COMMAND_EXPORT_ENCODING_KOREAN_EUC_KR: usize = 1034;
pub(super) const COMMAND_EXPORT_ENCODING_WINDOWS_1252: usize = 1035;
pub(super) const COMMAND_CLOSE_WINDOW: usize = 1036;
pub(super) const COMMAND_MOVE_UP: usize = 1037;
pub(super) const COMMAND_MOVE_DOWN: usize = 1038;
pub(super) const COMMAND_SAVE_DOCUMENT: usize = 1039;
pub(super) const COMMAND_THEME_LIGHT: usize = 1040;
pub(super) const COMMAND_THEME_CLASSIC_DARK: usize = 1041;
pub(super) const COMMAND_THEME_SEPIA_TEAL: usize = 1043;
pub(super) const COMMAND_THEME_GRAPHITE: usize = 1044;
pub(super) const COMMAND_THEME_FOREST: usize = 1045;
pub(super) const COMMAND_THEME_STEEL_BLUE: usize = 1046;
pub(super) const COMMAND_FIND_TEXT: usize = 1047;
pub(super) const COMMAND_REPLACE_TEXT: usize = 1048;
pub(super) const COMMAND_ABOUT: usize = 1049;
pub(super) const COMMAND_LANGUAGE_KOREAN: usize = 1050;
pub(super) const COMMAND_LANGUAGE_ENGLISH: usize = 1051;
pub(super) const COMMAND_EXPORT_ALL_TEXT: usize = 1052;
pub(super) const CONTROL_TREE_ID: usize = 2001;
pub(super) const CONTROL_EDITOR_ID: usize = 2002;
pub(super) const CONTROL_SEARCH_ID: usize = 2003;
pub(super) const CONTROL_TAB_ID: usize = 2004;
pub(super) const CONTROL_CARET_STATUS_ID: usize = 2005;
pub(super) const WM_APP_CLOSE_TAB: u32 = WM_APP + 1;
pub(super) const WM_APP_MOVE_TAB: u32 = WM_APP + 2;
pub(super) const WM_APP_REFRESH_TREE_AFTER_LABEL_EDIT: u32 = WM_APP + 3;
pub(super) const WM_APP_EDITOR_IME_START: u32 = WM_APP + 4;
pub(super) const WM_APP_EDITOR_IME_END: u32 = WM_APP + 5;
pub(super) const EN_CHANGE_NOTIFICATION_CODE: usize = 0x0300;
pub(super) const EM_SETCUEBANNER_EDIT_CONTROL: u32 = 0x1501;
pub(super) const EM_EXLIMITTEXT_RICH_EDIT: u32 = 0x0435;
pub(super) const EM_EXGETSEL_RICH_EDIT: u32 = WM_USER + 52;
pub(super) const EM_EXSETSEL_RICH_EDIT: u32 = WM_USER + 55;
pub(super) const EM_SETBKGNDCOLOR_RICH_EDIT: u32 = WM_USER + 67;
pub(super) const EM_SETCHARFORMAT_RICH_EDIT: u32 = WM_USER + 68;
pub(super) const EM_SETEVENTMASK_RICH_EDIT: u32 = 0x0445;
pub(super) const EM_LINEINDEX_EDIT_CONTROL: u32 = 0x00BB;
pub(super) const EM_LINEFROMCHAR_EDIT_CONTROL: u32 = 0x00C9;
pub(super) const EM_GETTEXTEX_RICH_EDIT: u32 = WM_USER + 94;
pub(super) const EM_GETTEXTLENGTHEX_RICH_EDIT: u32 = WM_USER + 95;
pub(super) const EM_SHOWSCROLLBAR_RICH_EDIT: u32 = WM_USER + 96;
pub(super) const EM_SETTEXTMODE_RICH_EDIT: u32 = WM_USER + 89;
pub(super) const EM_SETTARGETDEVICE_RICH_EDIT: u32 = WM_USER + 72;
pub(super) const SCF_DEFAULT_RICH_EDIT: WPARAM = 0x0000_0000;
pub(super) const SCF_SELECTION_RICH_EDIT: WPARAM = 0x0000_0001;
pub(super) const SCF_ALL_RICH_EDIT: WPARAM = 0x0000_0004;
pub(super) const CFM_BACKCOLOR_RICH_EDIT: u32 = 0x0400_0000;
pub(super) const CFM_COLOR_RICH_EDIT: u32 = 0x4000_0000;
pub(super) const CFE_AUTOBACKCOLOR_RICH_EDIT: u32 = CFM_BACKCOLOR_RICH_EDIT;
pub(super) const CFE_AUTOCOLOR_RICH_EDIT: u32 = CFM_COLOR_RICH_EDIT;
pub(super) const ENM_CHANGE_RICH_EDIT: LPARAM = 0x0000_0001;
pub(super) const ENM_SELCHANGE_RICH_EDIT: LPARAM = 0x0008_0000;
pub(super) const EN_SELCHANGE_RICH_EDIT: u32 = 0x0702;
pub(super) const GT_USECRLF_RICH_EDIT: u32 = 0x0000_0001;
pub(super) const GTL_USECRLF_RICH_EDIT: u32 = 0x0000_0001;
pub(super) const GTL_NUMCHARS_RICH_EDIT: u32 = 0x0000_0008;
pub(super) const RICH_EDIT_CP_UNICODE: u32 = 1200;
pub(super) const SS_RIGHT_STATIC_CONTROL: u32 = 0x0000_0002;
pub(super) const TM_PLAINTEXT_RICH_EDIT: WPARAM = 0x0000_0001;
pub(super) const SB_HORZ_SCROLLBAR: WPARAM = 0;
pub(super) const STANDARD_TEXT_CONTROL_TEXT_LIMIT: WPARAM = 0x7FFF_FFFE;
pub(super) const DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_MIB: usize = 64;
pub(super) const DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS: usize =
    DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_MIB * 1024 * 1024 / 2;
pub(super) const DOCUMENT_EDITOR_RICH_EDIT_TEXT_LIMIT: WPARAM =
    if DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS as WPARAM <= STANDARD_TEXT_CONTROL_TEXT_LIMIT {
        DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS as WPARAM
    } else {
        STANDARD_TEXT_CONTROL_TEXT_LIMIT
    };
pub(super) const VK_CONTROL_KEY: i32 = 0x11;
pub(super) const VK_A_KEY: WPARAM = 0x41;
pub(super) const VK_F_KEY: WPARAM = 0x46;
pub(super) const VK_H_KEY: WPARAM = 0x48;
pub(super) const VK_S_KEY: WPARAM = 0x53;
pub(super) const VK_N_KEY: WPARAM = 0x4E;
pub(super) const VK_W_KEY: WPARAM = 0x57;
pub(super) const VK_RETURN_KEY: WPARAM = 0x0D;
pub(super) const VK_ESCAPE_KEY: WPARAM = 0x1B;
pub(super) const VK_F2_KEY: WPARAM = 0x71;
pub(super) const VK_DELETE_KEY: WPARAM = 0x2E;
pub(super) const VK_MENU_KEY: i32 = 0x12;
pub(super) const VK_UP_KEY: WPARAM = 0x26;
pub(super) const VK_DOWN_KEY: WPARAM = 0x28;

pub(super) static EMPTY_TEXT: [u16; 1] = [0];

pub(super) fn last_win32_error(action: &'static str) -> AppError {
    let code = unsafe { GetLastError() };
    win32_error(action, code)
}

pub(super) fn last_win32_error_with_user_message(
    action: &'static str,
    user_message: PlatformUserMessage,
) -> AppError {
    let code = unsafe { GetLastError() };
    win32_error_with_user_message(action, user_message, code)
}

pub(super) fn win32_error(action: &'static str, code: u32) -> AppError {
    win32_error_with_user_message(action, PlatformUserMessage::Generic, code)
}

pub(super) fn win32_error_with_user_message(
    action: &'static str,
    user_message: PlatformUserMessage,
    code: u32,
) -> AppError {
    AppError::platform_with_user_message(
        action,
        user_message,
        format!(
            "Win32 error code {code}: {}",
            std::io::Error::from_raw_os_error(code as i32)
        ),
    )
}

pub(super) fn import_encoding_for_command(command_id: usize) -> Option<TextEncoding> {
    match command_id {
        COMMAND_IMPORT_ENCODING_AUTO => Some(TextEncoding::AutoDetect),
        COMMAND_IMPORT_ENCODING_UTF8 => Some(TextEncoding::Utf8),
        COMMAND_IMPORT_ENCODING_UTF8_BOM => Some(TextEncoding::Utf8WithBom),
        COMMAND_IMPORT_ENCODING_UTF16_LE_BOM => Some(TextEncoding::Utf16LeWithBom),
        COMMAND_IMPORT_ENCODING_UTF16_BE_BOM => Some(TextEncoding::Utf16BeWithBom),
        COMMAND_IMPORT_ENCODING_KOREAN_EUC_KR => Some(TextEncoding::KoreanEucKr),
        COMMAND_IMPORT_ENCODING_WINDOWS_1252 => Some(TextEncoding::Windows1252),
        _ => None,
    }
}

pub(super) fn export_encoding_for_command(command_id: usize) -> Option<TextEncoding> {
    match command_id {
        COMMAND_EXPORT_ENCODING_UTF8 => Some(TextEncoding::Utf8),
        COMMAND_EXPORT_ENCODING_UTF8_BOM => Some(TextEncoding::Utf8WithBom),
        COMMAND_EXPORT_ENCODING_UTF16_LE_BOM => Some(TextEncoding::Utf16LeWithBom),
        COMMAND_EXPORT_ENCODING_UTF16_BE_BOM => Some(TextEncoding::Utf16BeWithBom),
        COMMAND_EXPORT_ENCODING_KOREAN_EUC_KR => Some(TextEncoding::KoreanEucKr),
        COMMAND_EXPORT_ENCODING_WINDOWS_1252 => Some(TextEncoding::Windows1252),
        _ => None,
    }
}

pub(super) fn appearance_theme_for_command(command_id: usize) -> Option<AppearanceTheme> {
    match command_id {
        COMMAND_THEME_LIGHT => Some(AppearanceTheme::Light),
        COMMAND_THEME_CLASSIC_DARK => Some(AppearanceTheme::ClassicDark),
        COMMAND_THEME_SEPIA_TEAL => Some(AppearanceTheme::SepiaTeal),
        COMMAND_THEME_GRAPHITE => Some(AppearanceTheme::Graphite),
        COMMAND_THEME_FOREST => Some(AppearanceTheme::Forest),
        COMMAND_THEME_STEEL_BLUE => Some(AppearanceTheme::SteelBlue),
        _ => None,
    }
}

pub(super) fn ui_language_for_command(command_id: usize) -> Option<UiLanguage> {
    match command_id {
        COMMAND_LANGUAGE_KOREAN => Some(UiLanguage::Korean),
        COMMAND_LANGUAGE_ENGLISH => Some(UiLanguage::English),
        _ => None,
    }
}

pub(super) fn command_for_import_encoding(encoding: TextEncoding) -> Option<usize> {
    match encoding {
        TextEncoding::AutoDetect => Some(COMMAND_IMPORT_ENCODING_AUTO),
        TextEncoding::Utf8 => Some(COMMAND_IMPORT_ENCODING_UTF8),
        TextEncoding::Utf8WithBom => Some(COMMAND_IMPORT_ENCODING_UTF8_BOM),
        TextEncoding::Utf16LeWithBom => Some(COMMAND_IMPORT_ENCODING_UTF16_LE_BOM),
        TextEncoding::Utf16BeWithBom => Some(COMMAND_IMPORT_ENCODING_UTF16_BE_BOM),
        TextEncoding::KoreanEucKr => Some(COMMAND_IMPORT_ENCODING_KOREAN_EUC_KR),
        TextEncoding::Windows1252 => Some(COMMAND_IMPORT_ENCODING_WINDOWS_1252),
    }
}

pub(super) fn command_for_export_encoding(encoding: TextEncoding) -> Option<usize> {
    match encoding {
        TextEncoding::AutoDetect => None,
        TextEncoding::Utf8 => Some(COMMAND_EXPORT_ENCODING_UTF8),
        TextEncoding::Utf8WithBom => Some(COMMAND_EXPORT_ENCODING_UTF8_BOM),
        TextEncoding::Utf16LeWithBom => Some(COMMAND_EXPORT_ENCODING_UTF16_LE_BOM),
        TextEncoding::Utf16BeWithBom => Some(COMMAND_EXPORT_ENCODING_UTF16_BE_BOM),
        TextEncoding::KoreanEucKr => Some(COMMAND_EXPORT_ENCODING_KOREAN_EUC_KR),
        TextEncoding::Windows1252 => Some(COMMAND_EXPORT_ENCODING_WINDOWS_1252),
    }
}

pub(super) fn command_for_appearance_theme(theme: AppearanceTheme) -> usize {
    match theme {
        AppearanceTheme::Light => COMMAND_THEME_LIGHT,
        AppearanceTheme::ClassicDark => COMMAND_THEME_CLASSIC_DARK,
        AppearanceTheme::SepiaTeal => COMMAND_THEME_SEPIA_TEAL,
        AppearanceTheme::Graphite => COMMAND_THEME_GRAPHITE,
        AppearanceTheme::Forest => COMMAND_THEME_FOREST,
        AppearanceTheme::SteelBlue => COMMAND_THEME_STEEL_BLUE,
    }
}

pub(super) fn command_for_ui_language(language: UiLanguage) -> usize {
    match language {
        UiLanguage::Korean => COMMAND_LANGUAGE_KOREAN,
        UiLanguage::English => COMMAND_LANGUAGE_ENGLISH,
    }
}

pub(super) fn command_for_gui_command(command: GuiCommand) -> usize {
    match command {
        GuiCommand::SaveDocument => COMMAND_SAVE_DOCUMENT,
        GuiCommand::ImportText => COMMAND_IMPORT_TEXT,
        GuiCommand::ExportText => COMMAND_EXPORT_TEXT,
        GuiCommand::ExportAllText => COMMAND_EXPORT_ALL_TEXT,
        GuiCommand::CloseTab => COMMAND_CLOSE_TAB,
        GuiCommand::CloseWindow => COMMAND_CLOSE_WINDOW,
        GuiCommand::Undo => COMMAND_EDITOR_UNDO,
        GuiCommand::Cut => COMMAND_EDITOR_CUT,
        GuiCommand::Copy => COMMAND_EDITOR_COPY,
        GuiCommand::Paste => COMMAND_EDITOR_PASTE,
        GuiCommand::DeleteSelection => COMMAND_EDITOR_DELETE_SELECTION,
        GuiCommand::SelectAll => COMMAND_EDITOR_SELECT_ALL,
        GuiCommand::FindText => COMMAND_FIND_TEXT,
        GuiCommand::ReplaceText => COMMAND_REPLACE_TEXT,
        GuiCommand::NewDocument => COMMAND_NEW_DOCUMENT,
        GuiCommand::NewChildDocument => COMMAND_NEW_CHILD_DOCUMENT,
        GuiCommand::Rename => COMMAND_RENAME,
        GuiCommand::MoveUp => COMMAND_MOVE_UP,
        GuiCommand::MoveDown => COMMAND_MOVE_DOWN,
        GuiCommand::MoveToTrash => COMMAND_DELETE,
        GuiCommand::Restore => COMMAND_RESTORE,
        GuiCommand::DeletePermanently => COMMAND_DELETE_PERMANENTLY,
        GuiCommand::ShowActiveTree => COMMAND_SHOW_ACTIVE_TREE,
        GuiCommand::ShowTrash => COMMAND_SHOW_TRASH,
        GuiCommand::WordWrap => COMMAND_EDITOR_WORD_WRAP,
        GuiCommand::EditorFont => COMMAND_EDITOR_FONT,
        GuiCommand::About => COMMAND_ABOUT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn gui_command_contract_maps_to_unique_win32_commands() {
        let mut command_ids = HashSet::new();
        for command in GuiCommand::ALL {
            let command_id = command_for_gui_command(command);
            assert!(command_id >= 1000, "invalid command id for {command:?}");
            assert!(
                command_ids.insert(command_id),
                "duplicate Win32 command id {command_id} for {command:?}"
            );
        }
    }

    #[test]
    fn option_menu_commands_round_trip_and_do_not_overlap_gui_commands() {
        let mut command_ids = GuiCommand::ALL
            .iter()
            .map(|command| command_for_gui_command(*command))
            .collect::<HashSet<_>>();

        for encoding in TextEncoding::import_options() {
            let command_id = command_for_import_encoding(*encoding)
                .expect("every import encoding option should have a Win32 command id");
            assert!(
                command_ids.insert(command_id),
                "duplicate Win32 command id {command_id} for import encoding {encoding:?}"
            );
            assert_eq!(import_encoding_for_command(command_id), Some(*encoding));
            assert_eq!(export_encoding_for_command(command_id), None);
        }

        assert_eq!(command_for_export_encoding(TextEncoding::AutoDetect), None);
        for encoding in TextEncoding::export_options() {
            let command_id = command_for_export_encoding(*encoding)
                .expect("every export encoding option should have a Win32 command id");
            assert!(
                command_ids.insert(command_id),
                "duplicate Win32 command id {command_id} for export encoding {encoding:?}"
            );
            assert_eq!(export_encoding_for_command(command_id), Some(*encoding));
            assert_eq!(import_encoding_for_command(command_id), None);
        }

        for theme in AppearanceTheme::options() {
            let command_id = command_for_appearance_theme(*theme);
            assert!(
                command_ids.insert(command_id),
                "duplicate Win32 command id {command_id} for theme {theme:?}"
            );
            assert_eq!(appearance_theme_for_command(command_id), Some(*theme));
        }

        for language in UiLanguage::options() {
            let command_id = command_for_ui_language(*language);
            assert!(
                command_ids.insert(command_id),
                "duplicate Win32 command id {command_id} for language {language:?}"
            );
            assert_eq!(ui_language_for_command(command_id), Some(*language));
        }
    }

    #[test]
    fn rich_edit_editor_constants_match_plain_text_policy() {
        assert_eq!(WM_APP_EDITOR_IME_START, WM_APP + 4);
        assert_eq!(WM_APP_EDITOR_IME_END, WM_APP + 5);
        assert_eq!(EM_SETBKGNDCOLOR_RICH_EDIT, WM_USER + 67);
        assert_eq!(EM_SETCHARFORMAT_RICH_EDIT, WM_USER + 68);
        assert_eq!(EM_SETTEXTMODE_RICH_EDIT, WM_USER + 89);
        assert_eq!(EM_SETTARGETDEVICE_RICH_EDIT, WM_USER + 72);
        assert_eq!(EM_GETTEXTEX_RICH_EDIT, WM_USER + 94);
        assert_eq!(EM_GETTEXTLENGTHEX_RICH_EDIT, WM_USER + 95);
        assert_eq!(EM_SHOWSCROLLBAR_RICH_EDIT, WM_USER + 96);
        assert_eq!(EM_LINEINDEX_EDIT_CONTROL, 0x00BB);
        assert_eq!(EM_LINEFROMCHAR_EDIT_CONTROL, 0x00C9);
        assert_eq!(SCF_DEFAULT_RICH_EDIT, 0);
        assert_eq!(SCF_SELECTION_RICH_EDIT, 0x0000_0001);
        assert_eq!(SCF_ALL_RICH_EDIT, 0x0000_0004);
        assert_eq!(CFM_BACKCOLOR_RICH_EDIT, 0x0400_0000);
        assert_eq!(CFM_COLOR_RICH_EDIT, 0x4000_0000);
        assert_eq!(CFE_AUTOBACKCOLOR_RICH_EDIT, CFM_BACKCOLOR_RICH_EDIT);
        assert_eq!(CFE_AUTOCOLOR_RICH_EDIT, CFM_COLOR_RICH_EDIT);
        assert_eq!(TM_PLAINTEXT_RICH_EDIT, 0x0000_0001);
        assert_eq!(ENM_CHANGE_RICH_EDIT, 0x0000_0001);
        assert_eq!(ENM_SELCHANGE_RICH_EDIT, 0x0008_0000);
        assert_eq!(EN_SELCHANGE_RICH_EDIT, 0x0702);
        assert_eq!(GT_USECRLF_RICH_EDIT, 0x0000_0001);
        assert_eq!(GTL_USECRLF_RICH_EDIT, 0x0000_0001);
        assert_eq!(GTL_NUMCHARS_RICH_EDIT, 0x0000_0008);
        assert_eq!(RICH_EDIT_CP_UNICODE, 1200);
        assert_eq!(SS_RIGHT_STATIC_CONTROL, 0x0000_0002);
        assert_eq!(
            DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS * 2,
            DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_MIB * 1024 * 1024
        );
        assert_eq!(
            DOCUMENT_EDITOR_RICH_EDIT_TEXT_LIMIT,
            DOCUMENT_EDITOR_SYNC_TEXT_LIMIT_UTF16_UNITS as WPARAM
        );
        const {
            assert!(DOCUMENT_EDITOR_RICH_EDIT_TEXT_LIMIT < STANDARD_TEXT_CONTROL_TEXT_LIMIT);
        }
        assert_eq!(SB_HORZ_SCROLLBAR, 0);
    }
}
