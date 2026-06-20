use windows_sys::Win32::Foundation::HWND;

use super::super::font::{choose_editor_font, create_editor_font, set_editor_font};
use super::super::i18n::ui_text;
use super::super::menu::{rebuild_main_menu, update_menu_state};
use super::super::theme::{apply_window_theme, ThemeResources};
use super::super::tree::refresh_tree;
use super::super::window::{
    recreate_document_editor_for_word_wrap, set_search_cue_text, show_error_message, window_state,
};
use super::{refresh_tabs_for_state, update_window_title};
use crate::domain::{AppearanceTheme, TextEncoding, UiLanguage};
use crate::error::{AppError, PlatformUserMessage};

pub(in crate::platform::win32) unsafe fn set_import_encoding_from_menu(
    hwnd: HWND,
    encoding: TextEncoding,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("set import encoding", "window state was not attached")
    })?;
    let mut settings = state.app.ui_settings();
    settings.text_encoding.import_encoding = encoding;
    state.app.save_ui_settings(settings)?;
    update_menu_state(hwnd, &state)
}

pub(in crate::platform::win32) unsafe fn set_export_encoding_from_menu(
    hwnd: HWND,
    encoding: TextEncoding,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("set export encoding", "window state was not attached")
    })?;
    let mut settings = state.app.ui_settings();
    settings.text_encoding.export_encoding = encoding;
    state.app.save_ui_settings(settings)?;
    update_menu_state(hwnd, &state)
}

pub(in crate::platform::win32) unsafe fn set_appearance_theme_from_menu(
    hwnd: HWND,
    theme: AppearanceTheme,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("set appearance theme", "window state was not attached")
    })?;

    if state.app.ui_settings().appearance.theme == theme {
        update_menu_state(hwnd, &state)?;
        return Ok(());
    }

    let mut settings = state.app.ui_settings();
    settings.appearance.set_theme(theme);
    let next_theme_resources = ThemeResources::new(settings.appearance.theme)?;
    let current_find_match = state.current_find_match_highlight_range();
    state.store_editor_content_in_active_tab()?;
    state.app.save_ui_settings(settings)?;
    state.theme_resources = next_theme_resources;

    apply_window_theme(hwnd, &mut state);
    if let Some((start, end)) = current_find_match {
        state.show_current_find_match_in_editor(start, end)?;
    }
    update_menu_state(hwnd, &state)?;
    update_window_title(hwnd, &state)
}

pub(in crate::platform::win32) unsafe fn set_ui_language_from_menu(
    hwnd: HWND,
    language: UiLanguage,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform("set UI language", "window state was not attached"))?;

    if state.app.ui_settings().language == language {
        update_menu_state(hwnd, &state)?;
        return Ok(());
    }

    state.store_editor_content_in_active_tab()?;

    let preferred_node_id = state.selected_node_id;
    let mut settings = state.app.ui_settings();
    settings.language = language;
    state.app.save_ui_settings(settings)?;
    set_search_cue_text(state.search, language)?;
    rebuild_main_menu(hwnd, &state)?;
    refresh_tree(hwnd, &mut state, preferred_node_id)?;
    state.update_caret_status_from_editor()
}

pub(in crate::platform::win32) unsafe fn toggle_editor_word_wrap_from_menu(
    hwnd: HWND,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform("toggle word wrap", "window state was not attached"))?;

    state.store_editor_content_in_active_tab()?;

    let previous_word_wrap = state.app.ui_settings().editor.word_wrap;
    let mut next_settings = state.app.ui_settings();
    next_settings.editor.toggle_word_wrap();
    let next_word_wrap = next_settings.editor.word_wrap;

    recreate_document_editor_for_word_wrap(hwnd, &mut state, next_word_wrap)?;

    let mut settings = state.app.ui_settings();
    settings.editor.word_wrap = next_word_wrap;
    if let Err(error) = state.app.save_ui_settings(settings) {
        recreate_document_editor_for_word_wrap(hwnd, &mut state, previous_word_wrap)?;
        return Err(error);
    }

    refresh_tabs_for_state(&mut state)?;
    update_menu_state(hwnd, &state)?;
    update_window_title(hwnd, &state)
}

pub(in crate::platform::win32) unsafe fn choose_editor_font_from_menu(
    hwnd: HWND,
) -> Result<(), AppError> {
    let (current, language) = {
        let state = window_state(hwnd).ok_or_else(|| {
            AppError::platform_with_user_message(
                "choose editor font",
                PlatformUserMessage::Font,
                "window state was not attached",
            )
        })?;
        (
            state.app.ui_settings().editor_font,
            state.app.ui_settings().language,
        )
    };
    let Some(selected) = choose_editor_font(hwnd, &current, language)? else {
        return Ok(());
    };

    let used_fallback = {
        let mut state = window_state(hwnd).ok_or_else(|| {
            AppError::platform_with_user_message(
                "choose editor font",
                PlatformUserMessage::Font,
                "window state was not attached",
            )
        })?;
        state.store_editor_content_in_active_tab()?;

        let applied = create_editor_font(state.editor, &selected)?;
        let used_fallback = applied.used_fallback;
        let mut settings = state.app.ui_settings();
        settings.editor_font = applied.settings.clone();
        state.app.save_ui_settings(settings)?;

        set_editor_font(state.editor, &state.suppress_editor_change, &applied.handle);
        state.editor_font_handle = Some(applied.handle);
        state.show_active_tab_in_editor()?;
        apply_window_theme(hwnd, &mut state);
        used_fallback
    };

    if used_fallback {
        show_error_message("j3TreeText", ui_text(language).font_fallback());
    }

    Ok(())
}
