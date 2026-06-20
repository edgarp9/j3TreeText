use std::collections::HashMap;
use std::path::Path;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONINFORMATION, MB_OK};

use super::super::common::last_win32_error;
use super::super::file_dialog::{
    choose_export_text_file, choose_export_text_folder, choose_import_text_file,
};
use super::super::i18n::ui_text;
use super::super::menu::update_menu_state;
use super::super::state::WindowState;
use super::super::text::{
    normalize_editor_plain_text_in_place, prepare_normalized_document_editor_text_reusing,
    utf8_to_wide_null, DocumentEditorTextPrepareError,
};
use super::super::window::window_state;
use super::save_tabs::{prompt_unsaved_changes, save_current_document, SaveOutcome};
use super::{refresh_tabs_for_state, update_window_title};
use crate::domain::{DirtyTabDecision, TextEncoding, UiLanguage};
use crate::error::AppError;
use crate::infra::text_file::{
    encode_text_file_for_export, EncodedTextExport, TEXT_FILE_BYTE_LIMIT,
};

const TEXT_FILE_MIB_LIMIT: usize = TEXT_FILE_BYTE_LIMIT / 1024 / 1024;

pub(in crate::platform::win32) unsafe fn import_text_from_menu(hwnd: HWND) -> Result<(), AppError> {
    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform("import text", "window state was not attached"))?;
    ensure_active_editable_document_for_import(&state)?;

    let language = state.app.ui_settings().language;
    let encoding = state.app.ui_settings().text_encoding.import_encoding;
    let Some(path) = choose_import_text_file(hwnd, language)? else {
        return Ok(());
    };

    if !resolve_active_dirty_before_import(hwnd, &mut state)? {
        update_menu_state(hwnd, &state)?;
        return Ok(());
    }
    ensure_active_editable_document_for_import(&state)?;

    let mut decoded = state.app.import_text_file(&path, encoding)?;
    prepare_imported_text_for_editor_reusing(
        &mut decoded.content,
        language,
        &mut state.editor_text_utf16_buffer,
    )?;

    if !state.tabs.import_active_content(decoded.content) {
        return Err(AppError::user(ui_text(language).open_import_document()));
    }

    state.show_active_tab_in_editor_with_prepared_text()?;
    refresh_tabs_for_state(&mut state)?;
    update_menu_state(hwnd, &state)?;
    update_window_title(hwnd, &state)
}

pub(in crate::platform::win32) unsafe fn export_text_from_menu(hwnd: HWND) -> Result<(), AppError> {
    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform("export text", "window state was not attached"))?;
    state.store_editor_content_in_active_tab()?;
    let language = state.app.ui_settings().language;
    let encoding = state.app.ui_settings().text_encoding.export_encoding;
    let Some(path) = choose_export_text_file(hwnd, language)? else {
        return Ok(());
    };

    let export = match state.tabs.active() {
        Some(tab) => prepare_export_text_from_editor(&tab.content, encoding, language)?,
        None => return Err(AppError::user(ui_text(language).open_export_document())),
    };

    state.app.export_encoded_text_file(&path, &export)
}

pub(in crate::platform::win32) unsafe fn export_all_text_from_menu(
    hwnd: HWND,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform("export all text", "window state was not attached"))?;
    state.store_editor_content_in_active_tab()?;
    let language = state.app.ui_settings().language;
    let encoding = state.app.ui_settings().text_encoding.export_encoding;
    let Some(directory) = choose_export_text_folder(hwnd, language)? else {
        return Ok(());
    };
    let content_overrides = dirty_active_tab_content_overrides(&state);

    let count = state
        .app
        .export_all_text_files(&directory, encoding, &content_overrides)?;
    show_export_all_complete(hwnd, language, count, &directory)
}

fn dirty_active_tab_content_overrides(state: &WindowState) -> HashMap<i64, &str> {
    state
        .tabs
        .tabs()
        .iter()
        .filter(|tab| tab.editable && tab.dirty)
        .map(|tab| (tab.node_id, tab.content.as_str()))
        .collect()
}

unsafe fn show_export_all_complete(
    hwnd: HWND,
    language: UiLanguage,
    count: usize,
    directory: &Path,
) -> Result<(), AppError> {
    let text = ui_text(language);
    let path = directory.display().to_string();
    let title = utf8_to_wide_null(
        "convert export all completion title",
        text.export_all_complete_title(),
    )?;
    let message = text.export_all_complete_message(count, &path);
    let message = utf8_to_wide_null("convert export all completion message", &message)?;
    let result = MessageBoxW(
        hwnd,
        message.as_ptr(),
        title.as_ptr(),
        MB_OK | MB_ICONINFORMATION,
    );
    if result == 0 {
        return Err(last_win32_error("show export all completion message"));
    }

    Ok(())
}

fn prepare_imported_text_for_editor_reusing(
    content: &mut String,
    language: UiLanguage,
    buffer: &mut Vec<u16>,
) -> Result<(), AppError> {
    normalize_editor_plain_text_in_place(content);
    prepare_normalized_document_editor_text_reusing(content, buffer).map_err(|error| match error {
        DocumentEditorTextPrepareError::EmbeddedNul => {
            AppError::user(ui_text(language).imported_text_nul())
        }
        DocumentEditorTextPrepareError::TextLimitExceeded
        | DocumentEditorTextPrepareError::Win32ControlLimitExceeded => {
            AppError::user(ui_text(language).imported_text_too_large())
        }
    })
}

fn prepare_export_text_from_editor(
    content: &str,
    encoding: TextEncoding,
    language: UiLanguage,
) -> Result<EncodedTextExport, AppError> {
    encode_text_file_for_export(content, encoding).map_err(|error| match error {
        AppError::TextFileTooLarge { .. } => {
            AppError::user(ui_text(language).export_text_too_large(TEXT_FILE_MIB_LIMIT))
        }
        error => error,
    })
}

unsafe fn resolve_active_dirty_before_import(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<bool, AppError> {
    state.store_editor_content_in_active_tab()?;
    let Some(tab) = state.tabs.active() else {
        return Ok(false);
    };
    if !tab.dirty {
        return Ok(true);
    }

    let tab_title = tab.title.clone();
    match prompt_unsaved_changes(hwnd, Some(&tab_title), state.app.ui_settings().language)? {
        DirtyTabDecision::Save => match save_current_document(hwnd, state)? {
            SaveOutcome::NoChanges
            | SaveOutcome::Saved
            | SaveOutcome::Reloaded
            | SaveOutcome::SavedAsNewDocument => Ok(true),
            SaveOutcome::Canceled => Ok(false),
        },
        DirtyTabDecision::Discard => {
            if let Some(index) = state.tabs.active_index() {
                state.tabs.discard_tab_changes(index);
            }
            Ok(true)
        }
        DirtyTabDecision::Cancel => Ok(false),
    }
}

fn ensure_active_editable_document_for_import(state: &WindowState) -> Result<(), AppError> {
    match state.tabs.active() {
        Some(tab) if tab.editable => Ok(()),
        _ => Err(AppError::user(
            ui_text(state.app.ui_settings().language).open_import_document(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imported_text_with_embedded_nul_is_rejected_before_tab_mutation() {
        let mut content = "before\0after".to_owned();
        let mut buffer = Vec::new();
        let error = match prepare_imported_text_for_editor_reusing(
            &mut content,
            UiLanguage::Korean,
            &mut buffer,
        ) {
            Ok(()) => panic!("embedded NUL should be rejected"),
            Err(error) => error,
        };

        assert_eq!(
            error.user_message(),
            "가져온 텍스트에 지원하지 않는 NUL 문자가 있습니다. 해당 문자를 제거하세요."
        );
    }

    #[test]
    fn imported_text_without_embedded_nul_is_prepared_for_editor() {
        let mut content = "first\n둘째".to_owned();
        let mut buffer = Vec::new();

        prepare_imported_text_for_editor_reusing(&mut content, UiLanguage::Korean, &mut buffer)
            .expect("imported text should be prepared");

        assert_eq!(content, "first\r\n둘째");
        assert_eq!(
            buffer,
            "first\r\n둘째\0".encode_utf16().collect::<Vec<u16>>()
        );
    }
}
