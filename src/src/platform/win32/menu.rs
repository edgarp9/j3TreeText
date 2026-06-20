use std::ptr;

use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows_sys::Win32::UI::Controls::EM_CANUNDO;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CheckMenuItem, CreateMenu, CreatePopupMenu, DestroyMenu, DrawMenuBar,
    EnableMenuItem, GetCursorPos, GetMenu, SendMessageW, SetForegroundWindow, SetMenu,
    TrackPopupMenu, HMENU, MF_BYCOMMAND, MF_CHECKED, MF_ENABLED, MF_GRAYED, MF_POPUP, MF_SEPARATOR,
    MF_STRING, MF_UNCHECKED, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
};

use super::commands::handle_command;
use super::common::{
    command_for_appearance_theme, command_for_export_encoding, command_for_gui_command,
    command_for_import_encoding, command_for_ui_language, last_win32_error, COMMAND_CLOSE_TAB,
    COMMAND_DELETE, COMMAND_DELETE_PERMANENTLY, COMMAND_EDITOR_COPY, COMMAND_EDITOR_CUT,
    COMMAND_EDITOR_DELETE_SELECTION, COMMAND_EDITOR_PASTE, COMMAND_EDITOR_SELECT_ALL,
    COMMAND_EDITOR_UNDO, COMMAND_EDITOR_WORD_WRAP, COMMAND_FIND_TEXT, COMMAND_MOVE_DOWN,
    COMMAND_MOVE_UP, COMMAND_NEW_CHILD_DOCUMENT, COMMAND_NEW_DOCUMENT, COMMAND_RENAME,
    COMMAND_REPLACE_TEXT, COMMAND_RESTORE, COMMAND_SAVE_DOCUMENT, COMMAND_SHOW_ACTIVE_TREE,
    COMMAND_SHOW_TRASH,
};
use super::i18n::{ui_text, UiText};
use super::state::{EditorMenuState, MenuState, WindowState};
use super::text::{
    document_editor_plain_text_len_utf16, editor_selection_utf16, utf8_to_wide_null,
};
use super::tree::select_tree_item_at_screen_point;
use super::window::window_state;
use crate::domain::{AppearanceTheme, TextEncoding, UiLanguage};
use crate::error::AppError;
use crate::platform::gui::command_contract::{
    GuiCommand, GuiMenuEntry, GuiMenuKind, GuiOptionMenu, EDITOR_CONTEXT_MENU_ENTRIES,
    MAIN_MENU_SPECS, TREE_CONTEXT_MENU_ENTRIES,
};

pub(super) unsafe fn set_main_menu(hwnd: HWND, language: UiLanguage) -> Result<(), AppError> {
    let text = ui_text(language);
    let menu = CreateMenu();
    if menu.is_null() {
        return Err(last_win32_error("create main menu"));
    }

    for spec in MAIN_MENU_SPECS {
        let submenu = CreatePopupMenu();
        if submenu.is_null() {
            return Err(last_win32_error("create main submenu"));
        }
        append_gui_menu_entries(submenu, spec.entries, text)?;
        append_submenu(menu, submenu, menu_label(text, spec.kind))?;
    }

    if SetMenu(hwnd, menu) == 0 {
        return Err(last_win32_error("set main menu"));
    }

    Ok(())
}

unsafe fn append_gui_menu_entries(
    menu: HMENU,
    entries: &[GuiMenuEntry],
    text: UiText,
) -> Result<(), AppError> {
    for entry in entries {
        match *entry {
            GuiMenuEntry::Command(command) => {
                append_menu_item(
                    menu,
                    command_for_gui_command(command),
                    command_label(text, command),
                )?;
            }
            GuiMenuEntry::Separator => append_menu_separator(menu)?,
            GuiMenuEntry::OptionMenu(option_menu) => {
                let submenu = CreatePopupMenu();
                if submenu.is_null() {
                    return Err(last_win32_error("create option submenu"));
                }
                append_option_menu(submenu, option_menu, text)?;
                append_submenu(menu, submenu, option_menu_label(text, option_menu))?;
            }
        }
    }

    Ok(())
}

unsafe fn append_option_menu(
    menu: HMENU,
    option_menu: GuiOptionMenu,
    text: UiText,
) -> Result<(), AppError> {
    match option_menu {
        GuiOptionMenu::ImportEncoding => append_import_encoding_menu(menu, text),
        GuiOptionMenu::ExportEncoding => append_export_encoding_menu(menu, text),
        GuiOptionMenu::Theme => append_theme_menu(menu, text),
        GuiOptionMenu::Language => append_language_menu(menu),
    }
}

fn menu_label(text: UiText, menu: GuiMenuKind) -> &'static str {
    match menu {
        GuiMenuKind::File => text.menu_file(),
        GuiMenuKind::Edit => text.menu_edit(),
        GuiMenuKind::Document => text.menu_document(),
        GuiMenuKind::View => text.menu_view(),
        GuiMenuKind::Help => text.menu_help(),
    }
}

fn option_menu_label(text: UiText, option_menu: GuiOptionMenu) -> &'static str {
    match option_menu {
        GuiOptionMenu::ImportEncoding => text.import_encoding(),
        GuiOptionMenu::ExportEncoding => text.export_encoding(),
        GuiOptionMenu::Theme => text.theme(),
        GuiOptionMenu::Language => text.language(),
    }
}

fn command_label(text: UiText, command: GuiCommand) -> &'static str {
    match command {
        GuiCommand::SaveDocument => text.save_document(),
        GuiCommand::ImportText => text.import_text(),
        GuiCommand::ExportText => text.export_text(),
        GuiCommand::ExportAllText => text.export_all_text(),
        GuiCommand::CloseTab => text.close_tab(),
        GuiCommand::CloseWindow => text.close_window(),
        GuiCommand::Undo => text.undo(),
        GuiCommand::Cut => text.cut(),
        GuiCommand::Copy => text.copy(),
        GuiCommand::Paste => text.paste(),
        GuiCommand::DeleteSelection => text.delete_selection(),
        GuiCommand::SelectAll => text.select_all(),
        GuiCommand::FindText => text.find_text(),
        GuiCommand::ReplaceText => text.replace_text(),
        GuiCommand::NewDocument => text.new_document(),
        GuiCommand::NewChildDocument => text.new_child_document(),
        GuiCommand::Rename => text.rename(),
        GuiCommand::MoveUp => text.move_up(),
        GuiCommand::MoveDown => text.move_down(),
        GuiCommand::MoveToTrash => text.move_to_trash(),
        GuiCommand::Restore => text.restore(),
        GuiCommand::DeletePermanently => text.delete_permanently(),
        GuiCommand::ShowActiveTree => text.document_tree(),
        GuiCommand::ShowTrash => text.trash(),
        GuiCommand::WordWrap => text.word_wrap(),
        GuiCommand::EditorFont => text.editor_font(),
        GuiCommand::About => text.about_menu(),
    }
}

pub(super) unsafe fn rebuild_main_menu(hwnd: HWND, state: &WindowState) -> Result<(), AppError> {
    let old_menu = GetMenu(hwnd);
    set_main_menu(hwnd, state.app.ui_settings().language)?;
    if !old_menu.is_null() {
        DestroyMenu(old_menu);
    }
    update_menu_state(hwnd, state)
}

unsafe fn append_import_encoding_menu(menu: HMENU, text: UiText) -> Result<(), AppError> {
    for encoding in TextEncoding::import_options() {
        let Some(command_id) = command_for_import_encoding(*encoding) else {
            return Err(AppError::platform(
                "append import encoding menu",
                "import encoding has no command id",
            ));
        };
        append_menu_item(menu, command_id, text.text_encoding_name(*encoding))?;
    }

    Ok(())
}

unsafe fn append_export_encoding_menu(menu: HMENU, text: UiText) -> Result<(), AppError> {
    for encoding in TextEncoding::export_options() {
        let Some(command_id) = command_for_export_encoding(*encoding) else {
            return Err(AppError::platform(
                "append export encoding menu",
                "export encoding has no command id",
            ));
        };
        append_menu_item(menu, command_id, text.text_encoding_name(*encoding))?;
    }

    Ok(())
}

unsafe fn append_theme_menu(menu: HMENU, text: UiText) -> Result<(), AppError> {
    for theme in AppearanceTheme::options() {
        append_menu_item(
            menu,
            command_for_appearance_theme(*theme),
            text.theme_name(*theme),
        )?;
    }

    Ok(())
}

unsafe fn append_language_menu(menu: HMENU) -> Result<(), AppError> {
    for language in UiLanguage::options() {
        append_menu_item(
            menu,
            command_for_ui_language(*language),
            language.display_name(),
        )?;
    }

    Ok(())
}

pub(super) unsafe fn update_menu_state(hwnd: HWND, state: &WindowState) -> Result<(), AppError> {
    let menu = GetMenu(hwnd);
    if menu.is_null() {
        return Err(last_win32_error("get main menu"));
    }

    apply_menu_state(menu, state.menu_state())?;
    apply_editor_menu_state(menu, editor_context_menu_state(state)?)?;
    apply_text_encoding_menu_state(menu, state.app.ui_settings().text_encoding)?;
    apply_appearance_theme_menu_state(menu, state.app.ui_settings().appearance.theme)?;
    apply_ui_language_menu_state(menu, state.app.ui_settings().language)?;
    set_menu_item_checked(
        menu,
        COMMAND_EDITOR_WORD_WRAP,
        state.app.ui_settings().editor.word_wrap,
    )?;
    if DrawMenuBar(hwnd) == 0 {
        return Err(last_win32_error("redraw menu bar"));
    }

    Ok(())
}

pub(super) unsafe fn handle_context_menu(
    hwnd: HWND,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Result<bool, AppError> {
    let Some(mut state) = window_state(hwnd) else {
        return Ok(false);
    };

    let target = wparam as HWND;
    let point = context_menu_screen_point(lparam)?;
    let from_keyboard = lparam == -1;

    let command = if target == state.tree {
        show_tree_context_menu(hwnd, &mut state, point, from_keyboard)?
    } else if target == state.editor {
        show_editor_context_menu(hwnd, &state, point)?
    } else {
        return Ok(false);
    };

    drop(state);
    if let Some(command) = command {
        handle_command(hwnd, command);
    }
    Ok(true)
}

unsafe fn show_tree_context_menu(
    hwnd: HWND,
    state: &mut WindowState,
    point: POINT,
    from_keyboard: bool,
) -> Result<Option<WPARAM>, AppError> {
    if !from_keyboard && !select_tree_item_at_screen_point(hwnd, state, point)? {
        return Ok(None);
    }

    let text = ui_text(state.app.ui_settings().language);
    let menu = OwnedPopupMenu::new("create tree context menu")?;
    append_gui_menu_entries(menu.handle(), TREE_CONTEXT_MENU_ENTRIES, text)?;
    apply_tree_context_menu_state(menu.handle(), state.menu_state())?;

    Ok(track_context_menu(hwnd, menu.handle(), point))
}

unsafe fn show_editor_context_menu(
    hwnd: HWND,
    state: &WindowState,
    point: POINT,
) -> Result<Option<WPARAM>, AppError> {
    SetFocus(state.editor);

    let text = ui_text(state.app.ui_settings().language);
    let menu = OwnedPopupMenu::new("create editor context menu")?;
    append_gui_menu_entries(menu.handle(), EDITOR_CONTEXT_MENU_ENTRIES, text)?;
    apply_editor_context_menu_state(menu.handle(), editor_context_menu_state(state)?)?;

    Ok(track_context_menu(hwnd, menu.handle(), point))
}

unsafe fn context_menu_screen_point(lparam: LPARAM) -> Result<POINT, AppError> {
    if lparam == -1 {
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) == 0 {
            return Err(last_win32_error("read context menu cursor position"));
        }
        return Ok(point);
    }

    let value = lparam as u32;
    Ok(POINT {
        x: (value & 0xffff) as i16 as i32,
        y: ((value >> 16) & 0xffff) as i16 as i32,
    })
}

unsafe fn track_context_menu(hwnd: HWND, menu: HMENU, point: POINT) -> Option<WPARAM> {
    SetForegroundWindow(hwnd);
    let command = TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
        point.x,
        point.y,
        0,
        hwnd,
        ptr::null(),
    );

    (command > 0).then_some(command as WPARAM)
}

unsafe fn apply_tree_context_menu_state(menu: HMENU, state: MenuState) -> Result<(), AppError> {
    set_menu_item_enabled(
        menu,
        COMMAND_NEW_CHILD_DOCUMENT,
        state.new_child_document_enabled,
    )?;
    set_menu_item_enabled(menu, COMMAND_NEW_DOCUMENT, state.new_document_enabled)?;
    set_menu_item_enabled(menu, COMMAND_RENAME, state.rename_enabled)?;
    set_menu_item_enabled(menu, COMMAND_MOVE_UP, state.move_up_enabled)?;
    set_menu_item_enabled(menu, COMMAND_MOVE_DOWN, state.move_down_enabled)?;
    set_menu_item_enabled(menu, COMMAND_DELETE, state.delete_enabled)?;
    set_menu_item_enabled(menu, COMMAND_RESTORE, state.restore_enabled)?;
    set_menu_item_enabled(
        menu,
        COMMAND_DELETE_PERMANENTLY,
        state.delete_permanently_enabled,
    )?;

    Ok(())
}

unsafe fn editor_context_menu_state(state: &WindowState) -> Result<EditorMenuState, AppError> {
    let active_tab = state.tabs.active();
    let editable = active_tab.map(|tab| tab.editable).unwrap_or(false);
    let has_selection = editor_selection_utf16(state.editor)
        .map(|(start, end)| end > start)
        .unwrap_or(false);
    let can_undo = !state.editor.is_null() && SendMessageW(state.editor, EM_CANUNDO, 0, 0) != 0;
    let has_text = if state.editor.is_null() {
        false
    } else {
        document_editor_plain_text_len_utf16(state.editor)? > 0
    };

    Ok(EditorMenuState::for_context(
        active_tab.is_some(),
        editable,
        has_selection,
        can_undo,
        has_text,
    ))
}

unsafe fn apply_editor_context_menu_state(
    menu: HMENU,
    state: EditorMenuState,
) -> Result<(), AppError> {
    set_menu_item_enabled(menu, COMMAND_EDITOR_UNDO, state.undo_enabled)?;
    set_menu_item_enabled(menu, COMMAND_EDITOR_CUT, state.cut_enabled)?;
    set_menu_item_enabled(menu, COMMAND_EDITOR_COPY, state.copy_enabled)?;
    set_menu_item_enabled(menu, COMMAND_EDITOR_PASTE, state.paste_enabled)?;
    set_menu_item_enabled(menu, COMMAND_EDITOR_DELETE_SELECTION, state.delete_enabled)?;
    set_menu_item_enabled(menu, COMMAND_EDITOR_SELECT_ALL, state.select_all_enabled)?;

    Ok(())
}

unsafe fn apply_editor_menu_state(menu: HMENU, state: EditorMenuState) -> Result<(), AppError> {
    apply_editor_context_menu_state(menu, state)?;
    set_menu_item_enabled(menu, COMMAND_FIND_TEXT, state.find_replace_enabled)?;
    set_menu_item_enabled(menu, COMMAND_REPLACE_TEXT, state.find_replace_enabled)?;

    Ok(())
}

unsafe fn apply_text_encoding_menu_state(
    menu: HMENU,
    settings: crate::domain::TextEncodingSettings,
) -> Result<(), AppError> {
    for encoding in TextEncoding::import_options() {
        let Some(command_id) = command_for_import_encoding(*encoding) else {
            return Err(AppError::platform(
                "check import encoding menu",
                "import encoding has no command id",
            ));
        };
        set_menu_item_checked(menu, command_id, *encoding == settings.import_encoding)?;
    }

    for encoding in TextEncoding::export_options() {
        let Some(command_id) = command_for_export_encoding(*encoding) else {
            return Err(AppError::platform(
                "check export encoding menu",
                "export encoding has no command id",
            ));
        };
        set_menu_item_checked(menu, command_id, *encoding == settings.export_encoding)?;
    }

    Ok(())
}

unsafe fn apply_appearance_theme_menu_state(
    menu: HMENU,
    selected_theme: AppearanceTheme,
) -> Result<(), AppError> {
    for theme in AppearanceTheme::options() {
        set_menu_item_checked(
            menu,
            command_for_appearance_theme(*theme),
            *theme == selected_theme,
        )?;
    }

    Ok(())
}

unsafe fn apply_ui_language_menu_state(
    menu: HMENU,
    selected_language: UiLanguage,
) -> Result<(), AppError> {
    for language in UiLanguage::options() {
        set_menu_item_checked(
            menu,
            command_for_ui_language(*language),
            *language == selected_language,
        )?;
    }

    Ok(())
}

unsafe fn apply_menu_state(menu: HMENU, state: MenuState) -> Result<(), AppError> {
    set_menu_item_enabled(menu, COMMAND_SAVE_DOCUMENT, state.save_enabled)?;
    set_menu_item_enabled(
        menu,
        COMMAND_NEW_CHILD_DOCUMENT,
        state.new_child_document_enabled,
    )?;
    set_menu_item_enabled(menu, COMMAND_CLOSE_TAB, state.close_tab_enabled)?;
    set_menu_item_enabled(menu, COMMAND_NEW_DOCUMENT, state.new_document_enabled)?;
    set_menu_item_enabled(menu, COMMAND_RENAME, state.rename_enabled)?;
    set_menu_item_enabled(menu, COMMAND_MOVE_UP, state.move_up_enabled)?;
    set_menu_item_enabled(menu, COMMAND_MOVE_DOWN, state.move_down_enabled)?;
    set_menu_item_enabled(menu, COMMAND_DELETE, state.delete_enabled)?;
    set_menu_item_enabled(menu, COMMAND_RESTORE, state.restore_enabled)?;
    set_menu_item_enabled(
        menu,
        COMMAND_DELETE_PERMANENTLY,
        state.delete_permanently_enabled,
    )?;
    set_menu_item_checked(menu, COMMAND_SHOW_ACTIVE_TREE, state.active_tree_checked)?;
    set_menu_item_checked(menu, COMMAND_SHOW_TRASH, state.trash_checked)?;

    Ok(())
}

unsafe fn set_menu_item_enabled(
    menu: HMENU,
    command_id: usize,
    enabled: bool,
) -> Result<(), AppError> {
    let state_flag = if enabled { MF_ENABLED } else { MF_GRAYED };
    let result = EnableMenuItem(
        menu,
        menu_command_id(command_id)?,
        MF_BYCOMMAND | state_flag,
    );
    if result == -1 {
        return Err(last_win32_error("enable menu item"));
    }

    Ok(())
}

unsafe fn set_menu_item_checked(
    menu: HMENU,
    command_id: usize,
    checked: bool,
) -> Result<(), AppError> {
    let state_flag = if checked { MF_CHECKED } else { MF_UNCHECKED };
    let result = CheckMenuItem(
        menu,
        menu_command_id(command_id)?,
        MF_BYCOMMAND | state_flag,
    );
    if result == u32::MAX {
        return Err(last_win32_error("check menu item"));
    }

    Ok(())
}

fn menu_command_id(command_id: usize) -> Result<u32, AppError> {
    u32::try_from(command_id)
        .map_err(|_| AppError::platform("update menu item", "command id is out of range"))
}

unsafe fn append_menu_item(menu: HMENU, command_id: usize, text: &str) -> Result<(), AppError> {
    let text = utf8_to_wide_null("convert menu item text", text)?;
    if AppendMenuW(menu, MF_STRING, command_id, text.as_ptr()) == 0 {
        return Err(last_win32_error("append menu item"));
    }

    Ok(())
}

unsafe fn append_menu_separator(menu: HMENU) -> Result<(), AppError> {
    if AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null()) == 0 {
        return Err(last_win32_error("append menu separator"));
    }

    Ok(())
}

unsafe fn append_submenu(menu: HMENU, submenu: HMENU, text: &str) -> Result<(), AppError> {
    let text = utf8_to_wide_null("convert submenu text", text)?;
    if AppendMenuW(menu, MF_POPUP, submenu as usize, text.as_ptr()) == 0 {
        return Err(last_win32_error("append submenu"));
    }

    Ok(())
}

struct OwnedPopupMenu {
    handle: HMENU,
}

impl OwnedPopupMenu {
    unsafe fn new(action: &'static str) -> Result<Self, AppError> {
        let handle = CreatePopupMenu();
        if handle.is_null() {
            return Err(last_win32_error(action));
        }

        Ok(Self { handle })
    }

    fn handle(&self) -> HMENU {
        self.handle
    }
}

impl Drop for OwnedPopupMenu {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                DestroyMenu(self.handle);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    fn rust_function_body<'a>(source: &'a str, name: &str) -> &'a str {
        let signature = format!("fn {name}");
        let signature_start = source
            .find(&signature)
            .unwrap_or_else(|| panic!("function `{name}` must exist"));
        let body_start = source[signature_start..]
            .find('{')
            .map(|offset| signature_start + offset + 1)
            .unwrap_or_else(|| panic!("function `{name}` must have a body"));

        let mut depth = 1usize;
        for (offset, character) in source[body_start..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[body_start..body_start + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("function `{name}` body must be closed");
    }

    fn compact(source: &str) -> String {
        let compacted: String = source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        compacted.replace(",)?", ")?")
    }

    fn assert_contains_all(body: &str, expectations: &[&str]) {
        let body = compact(body);
        for expectation in expectations {
            let expectation = compact(expectation);
            assert!(
                body.contains(&expectation),
                "expected function body to contain `{expectation}`"
            );
        }
    }

    #[test]
    fn win32_main_menu_state_applies_every_command_availability_field() {
        let source = include_str!("menu.rs");
        let body = rust_function_body(source, "apply_menu_state");

        assert_contains_all(
            body,
            &[
                "set_menu_item_enabled(menu, COMMAND_SAVE_DOCUMENT, state.save_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_NEW_CHILD_DOCUMENT, state.new_child_document_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_CLOSE_TAB, state.close_tab_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_NEW_DOCUMENT, state.new_document_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_RENAME, state.rename_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_MOVE_UP, state.move_up_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_MOVE_DOWN, state.move_down_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_DELETE, state.delete_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_RESTORE, state.restore_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_DELETE_PERMANENTLY, state.delete_permanently_enabled)?;",
                "set_menu_item_checked(menu, COMMAND_SHOW_ACTIVE_TREE, state.active_tree_checked)?;",
                "set_menu_item_checked(menu, COMMAND_SHOW_TRASH, state.trash_checked)?;",
            ],
        );
    }

    #[test]
    fn win32_context_menus_apply_their_visible_availability_subsets() {
        let source = include_str!("menu.rs");
        let tree_body = rust_function_body(source, "apply_tree_context_menu_state");
        let editor_context_body = rust_function_body(source, "apply_editor_context_menu_state");
        let editor_menu_body = rust_function_body(source, "apply_editor_menu_state");

        assert_contains_all(
            tree_body,
            &[
                "set_menu_item_enabled(menu, COMMAND_NEW_CHILD_DOCUMENT, state.new_child_document_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_NEW_DOCUMENT, state.new_document_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_RENAME, state.rename_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_MOVE_UP, state.move_up_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_MOVE_DOWN, state.move_down_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_DELETE, state.delete_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_RESTORE, state.restore_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_DELETE_PERMANENTLY, state.delete_permanently_enabled)?;",
            ],
        );
        assert_contains_all(
            editor_context_body,
            &[
                "set_menu_item_enabled(menu, COMMAND_EDITOR_UNDO, state.undo_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_EDITOR_CUT, state.cut_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_EDITOR_COPY, state.copy_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_EDITOR_PASTE, state.paste_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_EDITOR_DELETE_SELECTION, state.delete_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_EDITOR_SELECT_ALL, state.select_all_enabled)?;",
            ],
        );
        assert_contains_all(
            editor_menu_body,
            &[
                "apply_editor_context_menu_state(menu, state)?;",
                "set_menu_item_enabled(menu, COMMAND_FIND_TEXT, state.find_replace_enabled)?;",
                "set_menu_item_enabled(menu, COMMAND_REPLACE_TEXT, state.find_replace_enabled)?;",
            ],
        );
    }
}
