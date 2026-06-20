use std::cell::Cell;
use std::ptr;

use windows_sys::core::{HRESULT, PCWSTR};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, S_OK, WPARAM};
use windows_sys::Win32::UI::Controls::{
    TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TDCBF_OK_BUTTON,
    TDF_ALLOW_DIALOG_CANCELLATION, TDF_ENABLE_HYPERLINKS, TDF_POSITION_RELATIVE_TO_WINDOW,
    TDN_HYPERLINK_CLICKED, TD_INFORMATION_ICON,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetWindowThreadProcessId, SendMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    HCBT_ACTIVATE, HHOOK, SW_SHOWNORMAL, WH_CBT, WM_CLEAR, WM_COPY, WM_CUT, WM_PASTE, WM_UNDO,
};

use super::super::common::{
    appearance_theme_for_command, export_encoding_for_command, import_encoding_for_command,
    last_win32_error, ui_language_for_command, COMMAND_ABOUT, COMMAND_CLOSE_TAB,
    COMMAND_CLOSE_WINDOW, COMMAND_DELETE, COMMAND_DELETE_PERMANENTLY, COMMAND_EDITOR_COPY,
    COMMAND_EDITOR_CUT, COMMAND_EDITOR_DELETE_SELECTION, COMMAND_EDITOR_FONT, COMMAND_EDITOR_PASTE,
    COMMAND_EDITOR_SELECT_ALL, COMMAND_EDITOR_UNDO, COMMAND_EDITOR_WORD_WRAP,
    COMMAND_EXPORT_ALL_TEXT, COMMAND_EXPORT_TEXT, COMMAND_FIND_TEXT, COMMAND_IMPORT_TEXT,
    COMMAND_MOVE_DOWN, COMMAND_MOVE_UP, COMMAND_NEW_CHILD_DOCUMENT, COMMAND_NEW_DOCUMENT,
    COMMAND_RENAME, COMMAND_REPLACE_TEXT, COMMAND_RESTORE, COMMAND_SAVE_DOCUMENT,
    COMMAND_SHOW_ACTIVE_TREE, COMMAND_SHOW_TRASH, CONTROL_EDITOR_ID, CONTROL_SEARCH_ID,
    EN_CHANGE_NOTIFICATION_CODE,
};
use super::super::i18n::ui_text;
use super::super::layout::center_window_over_owner;
use super::super::state::TreeMode;
use super::super::text::{
    document_editor_plain_text_len_utf16, select_editor_text_utf16, utf8_to_wide_null,
};
use super::super::window::{show_app_error, window_language, window_state};
use super::find_replace::{open_find_dialog_from_window, open_replace_dialog_from_window};
use super::save_tabs::{
    close_active_tab_from_window, close_window_from_menu, save_current_document_from_window,
};
use super::search::handle_search_changed;
use super::settings::{
    choose_editor_font_from_menu, set_appearance_theme_from_menu, set_export_encoding_from_menu,
    set_import_encoding_from_menu, set_ui_language_from_menu, toggle_editor_word_wrap_from_menu,
};
use super::text_io::{export_all_text_from_menu, export_text_from_menu, import_text_from_menu};
use super::tree_commands::{
    create_child_document_from_selection, create_sibling_document_from_selection,
    delete_selected_node, move_selected_node_within_parent, permanently_delete_selected_node,
    rename_selected_node, restore_selected_node, show_tree_mode,
};
use crate::domain::SiblingMoveDirection;
use crate::error::AppError;

thread_local! {
    static CENTERED_DIALOG_OWNER: Cell<HWND> = const { Cell::new(ptr::null_mut()) };
}

struct CenteredDialogHook {
    hook: HHOOK,
    previous_owner: HWND,
}

impl CenteredDialogHook {
    unsafe fn install(owner: HWND) -> Result<Self, AppError> {
        let thread_id = GetWindowThreadProcessId(owner, ptr::null_mut());
        if thread_id == 0 {
            return Err(last_win32_error("read owner window thread"));
        }

        let previous_owner = CENTERED_DIALOG_OWNER.with(|slot| {
            let previous = slot.get();
            slot.set(owner);
            previous
        });

        let hook = SetWindowsHookExW(
            WH_CBT,
            Some(center_message_box_hook_proc),
            ptr::null_mut(),
            thread_id,
        );
        if hook.is_null() {
            CENTERED_DIALOG_OWNER.with(|slot| slot.set(previous_owner));
            return Err(last_win32_error("install dialog placement hook"));
        }

        Ok(Self {
            hook,
            previous_owner,
        })
    }
}

impl Drop for CenteredDialogHook {
    fn drop(&mut self) {
        if !self.hook.is_null() {
            unsafe {
                UnhookWindowsHookEx(self.hook);
            }
        }

        CENTERED_DIALOG_OWNER.with(|slot| slot.set(self.previous_owner));
    }
}

unsafe extern "system" fn center_message_box_hook_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code == HCBT_ACTIVATE as i32 {
        CENTERED_DIALOG_OWNER.with(|slot| {
            let owner = slot.get();
            if !owner.is_null() {
                let _ = center_window_over_owner(wparam as HWND, owner);
            }
        });
    }

    CallNextHookEx(ptr::null_mut(), code, wparam, lparam)
}

unsafe fn handle_editor_changed(hwnd: HWND) -> Result<(), AppError> {
    let Some(mut state) = window_state(hwnd) else {
        return Ok(());
    };
    if state.suppress_editor_change.get() {
        return Ok(());
    }

    if state.editor_ime_composition_active() {
        state.mark_editor_ime_composition_changed();
        state.mark_editor_content_pending_from_view()?;
        return Ok(());
    }

    state.clear_current_find_match_highlight()?;
    state.mark_editor_content_pending_from_view()?;
    state.update_caret_status_from_editor()?;
    Ok(())
}

pub(in crate::platform::win32) unsafe fn handle_command(hwnd: HWND, wparam: WPARAM) {
    let command_id = wparam & 0xffff;
    let notification_code = (wparam >> 16) & 0xffff;
    if command_id == CONTROL_SEARCH_ID && notification_code == EN_CHANGE_NOTIFICATION_CODE {
        if let Err(error) = handle_search_changed(hwnd) {
            show_app_error(hwnd, &error);
        }
        return;
    }

    if command_id == CONTROL_EDITOR_ID && notification_code == EN_CHANGE_NOTIFICATION_CODE {
        if let Err(error) = handle_editor_changed(hwnd) {
            show_app_error(hwnd, &error);
        }
        return;
    }

    let result = if let Some(encoding) = import_encoding_for_command(command_id) {
        set_import_encoding_from_menu(hwnd, encoding)
    } else if let Some(encoding) = export_encoding_for_command(command_id) {
        set_export_encoding_from_menu(hwnd, encoding)
    } else if let Some(theme) = appearance_theme_for_command(command_id) {
        set_appearance_theme_from_menu(hwnd, theme)
    } else if let Some(language) = ui_language_for_command(command_id) {
        set_ui_language_from_menu(hwnd, language)
    } else {
        match command_id {
            COMMAND_SAVE_DOCUMENT => save_current_document_from_window(hwnd),
            COMMAND_IMPORT_TEXT => import_text_from_menu(hwnd),
            COMMAND_EXPORT_TEXT => export_text_from_menu(hwnd),
            COMMAND_EXPORT_ALL_TEXT => export_all_text_from_menu(hwnd),
            COMMAND_CLOSE_WINDOW => close_window_from_menu(hwnd),
            COMMAND_NEW_CHILD_DOCUMENT => create_child_document_from_selection(hwnd),
            COMMAND_NEW_DOCUMENT => create_sibling_document_from_selection(hwnd),
            COMMAND_RENAME => rename_selected_node(hwnd),
            COMMAND_MOVE_UP => move_selected_node_within_parent(hwnd, SiblingMoveDirection::Up),
            COMMAND_MOVE_DOWN => move_selected_node_within_parent(hwnd, SiblingMoveDirection::Down),
            COMMAND_DELETE => delete_selected_node(hwnd),
            COMMAND_SHOW_ACTIVE_TREE => show_tree_mode(hwnd, TreeMode::Active),
            COMMAND_SHOW_TRASH => show_tree_mode(hwnd, TreeMode::Trash),
            COMMAND_CLOSE_TAB => close_active_tab_from_window(hwnd),
            COMMAND_EDITOR_WORD_WRAP => toggle_editor_word_wrap_from_menu(hwnd),
            COMMAND_EDITOR_FONT => choose_editor_font_from_menu(hwnd),
            COMMAND_EDITOR_UNDO => send_editor_command(hwnd, WM_UNDO, true),
            COMMAND_EDITOR_CUT => send_editor_command(hwnd, WM_CUT, true),
            COMMAND_EDITOR_COPY => send_editor_command(hwnd, WM_COPY, false),
            COMMAND_EDITOR_PASTE => send_editor_command(hwnd, WM_PASTE, true),
            COMMAND_EDITOR_DELETE_SELECTION => send_editor_command(hwnd, WM_CLEAR, true),
            COMMAND_EDITOR_SELECT_ALL => select_all_editor_text(hwnd),
            COMMAND_FIND_TEXT => open_find_dialog_from_window(hwnd),
            COMMAND_REPLACE_TEXT => open_replace_dialog_from_window(hwnd),
            COMMAND_RESTORE => restore_selected_node(hwnd),
            COMMAND_DELETE_PERMANENTLY => permanently_delete_selected_node(hwnd),
            COMMAND_ABOUT => show_about_dialog(hwnd),
            _ => Ok(()),
        }
    };

    if let Err(error) = result {
        show_app_error(hwnd, &error);
    }
}

unsafe fn send_editor_command(
    hwnd: HWND,
    message: u32,
    requires_editable: bool,
) -> Result<(), AppError> {
    let editor = {
        let state = window_state(hwnd).ok_or_else(|| {
            AppError::platform("handle editor command", "window state was not attached")
        })?;
        let Some(tab) = state.tabs.active() else {
            return Ok(());
        };
        if requires_editable && !tab.editable {
            return Ok(());
        }
        state.editor
    };

    SetFocus(editor);
    SendMessageW(editor, message, 0, 0);
    refresh_caret_status(hwnd)?;
    Ok(())
}

pub(in crate::platform::win32) unsafe fn select_all_editor_text(
    hwnd: HWND,
) -> Result<(), AppError> {
    let editor = {
        let state = window_state(hwnd).ok_or_else(|| {
            AppError::platform("select all editor text", "window state was not attached")
        })?;
        if state.tabs.active().is_none() {
            return Ok(());
        }
        state.editor
    };

    SetFocus(editor);
    let text_len = document_editor_plain_text_len_utf16(editor)?;
    if text_len > 0 {
        select_editor_text_utf16(editor, 0, text_len)?;
    }
    refresh_caret_status(hwnd)?;
    Ok(())
}

unsafe fn refresh_caret_status(hwnd: HWND) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("refresh caret status", "window state was not attached")
    })?;
    state.update_caret_status_from_editor()
}

unsafe fn show_about_dialog(hwnd: HWND) -> Result<(), AppError> {
    let text = ui_text(window_language(hwnd));
    let title = utf8_to_wide_null("convert about title", text.about_title())?;
    let content = text.about_hyperlink_content(env!("CARGO_PKG_VERSION"));
    let content = utf8_to_wide_null("convert about content", &content)?;
    let mut callback_state = AboutDialogCallbackState::default();
    let config = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: hwnd,
        dwFlags: TDF_ENABLE_HYPERLINKS
            | TDF_ALLOW_DIALOG_CANCELLATION
            | TDF_POSITION_RELATIVE_TO_WINDOW,
        dwCommonButtons: TDCBF_OK_BUTTON,
        pszWindowTitle: title.as_ptr(),
        Anonymous1: TASKDIALOGCONFIG_0 {
            pszMainIcon: TD_INFORMATION_ICON,
        },
        pszContent: content.as_ptr(),
        pfCallback: Some(about_task_dialog_callback),
        lpCallbackData: &mut callback_state as *mut AboutDialogCallbackState as isize,
        ..Default::default()
    };

    let _placement_hook = CenteredDialogHook::install(hwnd)?;
    let task_result =
        TaskDialogIndirect(&config, ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
    if task_result < 0 {
        return Err(AppError::platform(
            "show about dialog",
            format!(
                "TaskDialogIndirect returned HRESULT 0x{:08X}",
                task_result as u32
            ),
        ));
    }
    if let Some(code) = callback_state.open_link_error {
        return Err(AppError::platform(
            "open about link",
            format!("ShellExecuteW returned {code}"),
        ));
    }

    Ok(())
}

#[derive(Default)]
struct AboutDialogCallbackState {
    open_link_error: Option<isize>,
}

unsafe extern "system" fn about_task_dialog_callback(
    hwnd: HWND,
    message: u32,
    _wparam: WPARAM,
    lparam: LPARAM,
    callback_data: isize,
) -> HRESULT {
    if message == TDN_HYPERLINK_CLICKED as u32 {
        let result = ShellExecuteW(
            hwnd,
            ptr::null(),
            lparam as PCWSTR,
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        ) as isize;
        if result <= 32 {
            let state = (callback_data as *mut AboutDialogCallbackState).as_mut();
            if let Some(state) = state {
                state.open_link_error = Some(result);
            }
        }
    }
    S_OK
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::gui::command_contract::GuiCommand;
    use crate::platform::win32::common::command_for_gui_command;

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
        source
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect()
    }

    fn expected_dispatch(command: GuiCommand) -> (&'static str, usize, &'static str) {
        match command {
            GuiCommand::SaveDocument => (
                "COMMAND_SAVE_DOCUMENT",
                COMMAND_SAVE_DOCUMENT,
                "COMMAND_SAVE_DOCUMENT => save_current_document_from_window(hwnd),",
            ),
            GuiCommand::ImportText => (
                "COMMAND_IMPORT_TEXT",
                COMMAND_IMPORT_TEXT,
                "COMMAND_IMPORT_TEXT => import_text_from_menu(hwnd),",
            ),
            GuiCommand::ExportText => (
                "COMMAND_EXPORT_TEXT",
                COMMAND_EXPORT_TEXT,
                "COMMAND_EXPORT_TEXT => export_text_from_menu(hwnd),",
            ),
            GuiCommand::ExportAllText => (
                "COMMAND_EXPORT_ALL_TEXT",
                COMMAND_EXPORT_ALL_TEXT,
                "COMMAND_EXPORT_ALL_TEXT => export_all_text_from_menu(hwnd),",
            ),
            GuiCommand::CloseTab => (
                "COMMAND_CLOSE_TAB",
                COMMAND_CLOSE_TAB,
                "COMMAND_CLOSE_TAB => close_active_tab_from_window(hwnd),",
            ),
            GuiCommand::CloseWindow => (
                "COMMAND_CLOSE_WINDOW",
                COMMAND_CLOSE_WINDOW,
                "COMMAND_CLOSE_WINDOW => close_window_from_menu(hwnd),",
            ),
            GuiCommand::Undo => (
                "COMMAND_EDITOR_UNDO",
                COMMAND_EDITOR_UNDO,
                "COMMAND_EDITOR_UNDO => send_editor_command(hwnd, WM_UNDO, true),",
            ),
            GuiCommand::Cut => (
                "COMMAND_EDITOR_CUT",
                COMMAND_EDITOR_CUT,
                "COMMAND_EDITOR_CUT => send_editor_command(hwnd, WM_CUT, true),",
            ),
            GuiCommand::Copy => (
                "COMMAND_EDITOR_COPY",
                COMMAND_EDITOR_COPY,
                "COMMAND_EDITOR_COPY => send_editor_command(hwnd, WM_COPY, false),",
            ),
            GuiCommand::Paste => (
                "COMMAND_EDITOR_PASTE",
                COMMAND_EDITOR_PASTE,
                "COMMAND_EDITOR_PASTE => send_editor_command(hwnd, WM_PASTE, true),",
            ),
            GuiCommand::DeleteSelection => (
                "COMMAND_EDITOR_DELETE_SELECTION",
                COMMAND_EDITOR_DELETE_SELECTION,
                "COMMAND_EDITOR_DELETE_SELECTION => send_editor_command(hwnd, WM_CLEAR, true),",
            ),
            GuiCommand::SelectAll => (
                "COMMAND_EDITOR_SELECT_ALL",
                COMMAND_EDITOR_SELECT_ALL,
                "COMMAND_EDITOR_SELECT_ALL => select_all_editor_text(hwnd),",
            ),
            GuiCommand::FindText => (
                "COMMAND_FIND_TEXT",
                COMMAND_FIND_TEXT,
                "COMMAND_FIND_TEXT => open_find_dialog_from_window(hwnd),",
            ),
            GuiCommand::ReplaceText => (
                "COMMAND_REPLACE_TEXT",
                COMMAND_REPLACE_TEXT,
                "COMMAND_REPLACE_TEXT => open_replace_dialog_from_window(hwnd),",
            ),
            GuiCommand::NewDocument => (
                "COMMAND_NEW_DOCUMENT",
                COMMAND_NEW_DOCUMENT,
                "COMMAND_NEW_DOCUMENT => create_sibling_document_from_selection(hwnd),",
            ),
            GuiCommand::NewChildDocument => (
                "COMMAND_NEW_CHILD_DOCUMENT",
                COMMAND_NEW_CHILD_DOCUMENT,
                "COMMAND_NEW_CHILD_DOCUMENT => create_child_document_from_selection(hwnd),",
            ),
            GuiCommand::Rename => (
                "COMMAND_RENAME",
                COMMAND_RENAME,
                "COMMAND_RENAME => rename_selected_node(hwnd),",
            ),
            GuiCommand::MoveUp => (
                "COMMAND_MOVE_UP",
                COMMAND_MOVE_UP,
                "COMMAND_MOVE_UP => move_selected_node_within_parent(hwnd, SiblingMoveDirection::Up),",
            ),
            GuiCommand::MoveDown => (
                "COMMAND_MOVE_DOWN",
                COMMAND_MOVE_DOWN,
                "COMMAND_MOVE_DOWN => move_selected_node_within_parent(hwnd, SiblingMoveDirection::Down),",
            ),
            GuiCommand::MoveToTrash => (
                "COMMAND_DELETE",
                COMMAND_DELETE,
                "COMMAND_DELETE => delete_selected_node(hwnd),",
            ),
            GuiCommand::Restore => (
                "COMMAND_RESTORE",
                COMMAND_RESTORE,
                "COMMAND_RESTORE => restore_selected_node(hwnd),",
            ),
            GuiCommand::DeletePermanently => (
                "COMMAND_DELETE_PERMANENTLY",
                COMMAND_DELETE_PERMANENTLY,
                "COMMAND_DELETE_PERMANENTLY => permanently_delete_selected_node(hwnd),",
            ),
            GuiCommand::ShowActiveTree => (
                "COMMAND_SHOW_ACTIVE_TREE",
                COMMAND_SHOW_ACTIVE_TREE,
                "COMMAND_SHOW_ACTIVE_TREE => show_tree_mode(hwnd, TreeMode::Active),",
            ),
            GuiCommand::ShowTrash => (
                "COMMAND_SHOW_TRASH",
                COMMAND_SHOW_TRASH,
                "COMMAND_SHOW_TRASH => show_tree_mode(hwnd, TreeMode::Trash),",
            ),
            GuiCommand::WordWrap => (
                "COMMAND_EDITOR_WORD_WRAP",
                COMMAND_EDITOR_WORD_WRAP,
                "COMMAND_EDITOR_WORD_WRAP => toggle_editor_word_wrap_from_menu(hwnd),",
            ),
            GuiCommand::EditorFont => (
                "COMMAND_EDITOR_FONT",
                COMMAND_EDITOR_FONT,
                "COMMAND_EDITOR_FONT => choose_editor_font_from_menu(hwnd),",
            ),
            GuiCommand::About => (
                "COMMAND_ABOUT",
                COMMAND_ABOUT,
                "COMMAND_ABOUT => show_about_dialog(hwnd),",
            ),
        }
    }

    #[test]
    fn win32_handle_command_dispatches_every_gui_command() {
        let body = compact(rust_function_body(
            include_str!("dispatch.rs"),
            "handle_command",
        ));

        for command in GuiCommand::ALL {
            let (constant_name, command_id, arm) = expected_dispatch(command);
            assert_eq!(
                command_for_gui_command(command),
                command_id,
                "{command:?} must keep using {constant_name}"
            );
            assert!(
                body.contains(&compact(arm)),
                "{command:?} must be dispatched by handle_command through {constant_name}"
            );
        }
    }

    #[test]
    fn win32_handle_command_routes_option_menus_before_command_match() {
        let body = compact(rust_function_body(
            include_str!("dispatch.rs"),
            "handle_command",
        ));

        for snippet in [
            "if let Some(encoding) = import_encoding_for_command(command_id)",
            "set_import_encoding_from_menu(hwnd, encoding)",
            "else if let Some(encoding) = export_encoding_for_command(command_id)",
            "set_export_encoding_from_menu(hwnd, encoding)",
            "else if let Some(theme) = appearance_theme_for_command(command_id)",
            "set_appearance_theme_from_menu(hwnd, theme)",
            "else if let Some(language) = ui_language_for_command(command_id)",
            "set_ui_language_from_menu(hwnd, language)",
            "else { match command_id",
        ] {
            assert!(
                body.contains(&compact(snippet)),
                "handle_command must keep option menu dispatch snippet `{snippet}`"
            );
        }
    }
}
