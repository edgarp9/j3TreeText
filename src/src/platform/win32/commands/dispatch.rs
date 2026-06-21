use std::ptr;

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, LoadLibraryW};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, IsWindowEnabled, SetActiveWindow, SetFocus,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    IsChild, IsDialogMessageW, IsWindow, LoadCursorW, PostQuitMessage, RegisterClassW,
    SendMessageW, SetWindowTextW, ShowWindow, TranslateMessage, BS_DEFPUSHBUTTON, ES_AUTOHSCROLL,
    ES_AUTOVSCROLL, ES_MULTILINE, ES_NOHIDESEL, ES_READONLY, ES_WANTRETURN, HMENU, IDC_ARROW, MSG,
    SW_SHOW, SW_SHOWNORMAL, WM_CLEAR, WM_CLOSE, WM_COMMAND, WM_COPY, WM_CUT, WM_KEYDOWN, WM_PASTE,
    WM_UNDO, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_CLIPSIBLINGS, WS_EX_CLIENTEDGE,
    WS_EX_CONTROLPARENT, WS_EX_DLGMODALFRAME, WS_HSCROLL, WS_POPUP, WS_SYSMENU, WS_TABSTOP,
    WS_VISIBLE, WS_VSCROLL,
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
    DOCUMENT_EDITOR_RICH_EDIT_CLASS_NAME, DOCUMENT_EDITOR_RICH_EDIT_TEXT_LIMIT, EMPTY_TEXT,
    EM_EXLIMITTEXT_RICH_EDIT, EM_SETTEXTMODE_RICH_EDIT, EN_CHANGE_NOTIFICATION_CODE,
    RICH_EDIT_MODULE_NAME, TM_PLAINTEXT_RICH_EDIT, VK_ESCAPE_KEY,
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
use crate::domain::{app_about_title, app_version_label, SiblingMoveDirection, APP_AUTHOR_URL};
use crate::error::AppError;

const ABOUT_DIALOG_CLASS_NAME: &str = "j3TreeTextAboutDialogClass";
const STATIC_CONTROL_CLASS_NAME: &str = "STATIC";
const BUTTON_CONTROL_CLASS_NAME: &str = "BUTTON";
const ABOUT_DIALOG_WIDTH: i32 = 540;
const ABOUT_DIALOG_HEIGHT: i32 = 320;
const ABOUT_DIALOG_MARGIN: i32 = 12;
const ABOUT_DIALOG_CONTROL_GAP: i32 = 8;
const ABOUT_VERSION_LABEL_HEIGHT: i32 = 22;
const ABOUT_BUTTON_HEIGHT: i32 = 26;
const ABOUT_OK_BUTTON_WIDTH: i32 = 80;
const ABOUT_LINK_CONTROL_ID: usize = 1;
const ABOUT_OK_CONTROL_ID: usize = 2;

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
    let title = app_about_title();
    let title = utf8_to_wide_null("convert about title", &title)?;
    let version_label = app_version_label();
    let version_label = utf8_to_wide_null("convert about version label", &version_label)?;
    let content = text.about_message(env!("CARGO_PKG_VERSION"));
    let content = utf8_to_wide_null("convert about content", &content)?;
    let project_url = utf8_to_wide_null("convert about project URL", APP_AUTHOR_URL)?;

    let (dialog_width, dialog_height) = about_dialog_size(hwnd);
    show_about_modal_dialog(
        hwnd,
        &title,
        &version_label,
        &content,
        &project_url,
        text.ok_button(),
        dialog_width,
        dialog_height,
    )?;

    Ok(())
}

struct AboutDialog {
    hwnd: HWND,
    ok_button: HWND,
}

unsafe fn show_about_modal_dialog(
    owner: HWND,
    title: &[u16],
    version_label: &[u16],
    content: &[u16],
    project_url: &[u16],
    ok_button_text: &str,
    dialog_width: i32,
    dialog_height: i32,
) -> Result<(), AppError> {
    let ok_button_text = utf8_to_wide_null("convert about OK button text", ok_button_text)?;
    let dialog = create_about_dialog(
        owner,
        title,
        version_label,
        content,
        project_url,
        &ok_button_text,
        dialog_width,
        dialog_height,
    )?;
    if let Err(error) = center_window_over_owner(dialog.hwnd, owner) {
        DestroyWindow(dialog.hwnd);
        return Err(error);
    }

    let _disabled_owner = DisabledOwnerWindow::disable(owner);
    ShowWindow(dialog.hwnd, SW_SHOW);
    SetFocus(dialog.ok_button);
    run_modal_dialog_loop(dialog.hwnd, "read about dialog message")
}

unsafe fn create_about_dialog(
    owner: HWND,
    title: &[u16],
    version_label: &[u16],
    content: &[u16],
    project_url: &[u16],
    ok_button_text: &[u16],
    dialog_width: i32,
    dialog_height: i32,
) -> Result<AboutDialog, AppError> {
    register_about_dialog_class()?;
    ensure_dialog_rich_edit_module_loaded()?;

    let instance = GetModuleHandleW(ptr::null());
    if instance.is_null() {
        return Err(last_win32_error("get module handle for about dialog"));
    }

    let class_name = utf8_to_wide_null("convert about dialog class name", ABOUT_DIALOG_CLASS_NAME)?;
    let hwnd = CreateWindowExW(
        WS_EX_DLGMODALFRAME | WS_EX_CONTROLPARENT,
        class_name.as_ptr(),
        title.as_ptr(),
        about_dialog_style(),
        0,
        0,
        dialog_width,
        dialog_height,
        owner,
        ptr::null_mut(),
        instance,
        ptr::null(),
    );
    if hwnd.is_null() {
        return Err(last_win32_error("create about dialog"));
    }

    let result = create_about_dialog_controls(
        hwnd,
        instance,
        version_label,
        content,
        project_url,
        ok_button_text,
    )
    .map(|ok_button| AboutDialog { hwnd, ok_button });
    if result.is_err() {
        DestroyWindow(hwnd);
    }
    result
}

unsafe fn about_dialog_size(owner: HWND) -> (i32, i32) {
    window_state(owner)
        .map(|state| {
            (
                state.ui_scale.px(ABOUT_DIALOG_WIDTH),
                state.ui_scale.px(ABOUT_DIALOG_HEIGHT),
            )
        })
        .unwrap_or((ABOUT_DIALOG_WIDTH, ABOUT_DIALOG_HEIGHT))
}

unsafe fn register_about_dialog_class() -> Result<(), AppError> {
    let instance = GetModuleHandleW(ptr::null());
    if instance.is_null() {
        return Err(last_win32_error("get module handle for about dialog class"));
    }

    let class_name = utf8_to_wide_null("convert about dialog class name", ABOUT_DIALOG_CLASS_NAME)?;
    let cursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);
    if cursor.is_null() {
        return Err(last_win32_error("load about dialog cursor"));
    }

    let window_class = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(about_dialog_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: ptr::null_mut(),
        hCursor: cursor,
        hbrBackground: (COLOR_WINDOW + 1) as usize as HBRUSH,
        lpszMenuName: ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };

    let atom = RegisterClassW(&window_class);
    if atom == 0 {
        let code = GetLastError();
        if code != ERROR_CLASS_ALREADY_EXISTS {
            return Err(last_win32_error("register about dialog class"));
        }
    }

    Ok(())
}

unsafe fn create_about_dialog_controls(
    parent: HWND,
    instance: windows_sys::Win32::Foundation::HINSTANCE,
    version_label: &[u16],
    content: &[u16],
    project_url: &[u16],
    ok_button_text: &[u16],
) -> Result<HWND, AppError> {
    let mut rect = RECT::default();
    if GetClientRect(parent, &mut rect) == 0 {
        return Err(last_win32_error("read about dialog client area"));
    }

    let client_width = rect.right - rect.left;
    let client_height = rect.bottom - rect.top;
    let content_width = (client_width - ABOUT_DIALOG_MARGIN * 2).max(0);
    let text_y = ABOUT_DIALOG_MARGIN + ABOUT_VERSION_LABEL_HEIGHT + ABOUT_DIALOG_CONTROL_GAP;
    let button_y = (client_height - ABOUT_DIALOG_MARGIN - ABOUT_BUTTON_HEIGHT).max(0);
    let text_height = (button_y - ABOUT_DIALOG_CONTROL_GAP - text_y).max(0);

    let static_class =
        utf8_to_wide_null("convert about static class name", STATIC_CONTROL_CLASS_NAME)?;
    let version = CreateWindowExW(
        0,
        static_class.as_ptr(),
        version_label.as_ptr(),
        about_label_style(),
        ABOUT_DIALOG_MARGIN,
        ABOUT_DIALOG_MARGIN,
        content_width,
        ABOUT_VERSION_LABEL_HEIGHT,
        parent,
        ptr::null_mut(),
        instance,
        ptr::null(),
    );
    if version.is_null() {
        return Err(last_win32_error("create about version label"));
    }

    let rich_edit_class = utf8_to_wide_null(
        "convert about Rich Edit class name",
        DOCUMENT_EDITOR_RICH_EDIT_CLASS_NAME,
    )?;
    let about_text = CreateWindowExW(
        WS_EX_CLIENTEDGE,
        rich_edit_class.as_ptr(),
        EMPTY_TEXT.as_ptr(),
        dialog_text_control_style(),
        ABOUT_DIALOG_MARGIN,
        text_y,
        content_width,
        text_height,
        parent,
        ptr::null_mut(),
        instance,
        ptr::null(),
    );
    if about_text.is_null() {
        return Err(last_win32_error("create about text control"));
    }
    configure_dialog_text_control(about_text, content)?;

    let button_class =
        utf8_to_wide_null("convert about button class name", BUTTON_CONTROL_CLASS_NAME)?;
    let ok_button_x = (client_width - ABOUT_DIALOG_MARGIN - ABOUT_OK_BUTTON_WIDTH).max(0);
    let link_width = (ok_button_x - ABOUT_DIALOG_MARGIN * 2 - ABOUT_DIALOG_CONTROL_GAP).max(0);
    let link_button = CreateWindowExW(
        0,
        button_class.as_ptr(),
        project_url.as_ptr(),
        about_link_button_style(),
        ABOUT_DIALOG_MARGIN,
        button_y,
        link_width,
        ABOUT_BUTTON_HEIGHT,
        parent,
        ABOUT_LINK_CONTROL_ID as HMENU,
        instance,
        ptr::null(),
    );
    if link_button.is_null() {
        return Err(last_win32_error("create about project URL button"));
    }

    let ok_button = CreateWindowExW(
        0,
        button_class.as_ptr(),
        ok_button_text.as_ptr(),
        dialog_ok_button_style(),
        ok_button_x,
        button_y,
        ABOUT_OK_BUTTON_WIDTH,
        ABOUT_BUTTON_HEIGHT,
        parent,
        ABOUT_OK_CONTROL_ID as HMENU,
        instance,
        ptr::null(),
    );
    if ok_button.is_null() {
        return Err(last_win32_error("create about OK button"));
    }

    Ok(ok_button)
}

fn about_dialog_style() -> u32 {
    WS_POPUP | WS_CAPTION | WS_SYSMENU
}

fn about_label_style() -> u32 {
    WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS
}

fn about_link_button_style() -> u32 {
    WS_CHILD | WS_VISIBLE | WS_TABSTOP
}

unsafe fn open_about_project_url(owner: HWND) -> Result<(), AppError> {
    let operation = utf8_to_wide_null("convert about link operation", "open")?;
    let url = utf8_to_wide_null("convert about link URL", APP_AUTHOR_URL)?;
    let result = ShellExecuteW(
        owner,
        operation.as_ptr(),
        url.as_ptr(),
        ptr::null(),
        ptr::null(),
        SW_SHOWNORMAL,
    ) as isize;

    if result <= 32 {
        Err(AppError::platform(
            "open about link",
            format!("ShellExecuteW returned {result}"),
        ))
    } else {
        Ok(())
    }
}

unsafe extern "system" fn about_dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_COMMAND => {
            let command_id = wparam & 0xffff;
            match command_id {
                ABOUT_LINK_CONTROL_ID => {
                    if let Err(error) = open_about_project_url(hwnd) {
                        show_app_error(hwnd, &error);
                    }
                    return 0;
                }
                ABOUT_OK_CONTROL_ID => {
                    DestroyWindow(hwnd);
                    return 0;
                }
                _ => {}
            }
        }
        WM_CLOSE => {
            DestroyWindow(hwnd);
            return 0;
        }
        _ => {}
    }

    DefWindowProcW(hwnd, message, wparam, lparam)
}

struct DisabledOwnerWindow {
    hwnd: HWND,
    should_reenable: bool,
}

impl DisabledOwnerWindow {
    unsafe fn disable(hwnd: HWND) -> Self {
        if hwnd.is_null() {
            return Self {
                hwnd,
                should_reenable: false,
            };
        }

        let was_enabled = IsWindowEnabled(hwnd) != 0;
        EnableWindow(hwnd, 0);
        Self {
            hwnd,
            should_reenable: was_enabled,
        }
    }
}

impl Drop for DisabledOwnerWindow {
    fn drop(&mut self) {
        if self.should_reenable && !self.hwnd.is_null() {
            unsafe {
                EnableWindow(self.hwnd, 1);
                SetActiveWindow(self.hwnd);
            }
        }
    }
}

unsafe fn ensure_dialog_rich_edit_module_loaded() -> Result<(), AppError> {
    let library_name = utf8_to_wide_null(
        "convert dialog Rich Edit module name",
        RICH_EDIT_MODULE_NAME,
    )?;
    let existing = GetModuleHandleW(library_name.as_ptr());
    if !existing.is_null() {
        return Ok(());
    }

    let module = LoadLibraryW(library_name.as_ptr());
    if module.is_null() {
        return Err(last_win32_error("load dialog Rich Edit library"));
    }

    Ok(())
}

unsafe fn configure_dialog_text_control(hwnd: HWND, content: &[u16]) -> Result<(), AppError> {
    let text_mode_result = SendMessageW(hwnd, EM_SETTEXTMODE_RICH_EDIT, TM_PLAINTEXT_RICH_EDIT, 0);
    if text_mode_result != 0 {
        return Err(AppError::platform(
            "set dialog Rich Edit plain text mode",
            format!("Rich Edit rejected plain text mode with result {text_mode_result}"),
        ));
    }

    SendMessageW(
        hwnd,
        EM_EXLIMITTEXT_RICH_EDIT,
        0,
        DOCUMENT_EDITOR_RICH_EDIT_TEXT_LIMIT as LPARAM,
    );

    if SetWindowTextW(hwnd, content.as_ptr()) == 0 {
        return Err(last_win32_error("set dialog text"));
    }

    Ok(())
}

fn dialog_text_control_style() -> u32 {
    WS_CHILD
        | WS_VISIBLE
        | WS_TABSTOP
        | WS_CLIPSIBLINGS
        | WS_VSCROLL
        | WS_HSCROLL
        | ES_MULTILINE as u32
        | ES_AUTOVSCROLL as u32
        | ES_AUTOHSCROLL as u32
        | ES_READONLY as u32
        | ES_NOHIDESEL as u32
        | ES_WANTRETURN as u32
}

fn dialog_ok_button_style() -> u32 {
    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32
}

unsafe fn run_modal_dialog_loop(
    dialog: HWND,
    read_message_action: &'static str,
) -> Result<(), AppError> {
    let mut message: MSG = std::mem::zeroed();

    while IsWindow(dialog) != 0 {
        let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
        if result == -1 {
            return Err(last_win32_error(read_message_action));
        }
        if result == 0 {
            PostQuitMessage(message.wParam as i32);
            return Ok(());
        }

        if modal_dialog_handles_escape(dialog, &message) {
            continue;
        }

        if IsDialogMessageW(dialog, &message) != 0 {
            continue;
        }

        TranslateMessage(&message);
        DispatchMessageW(&message);
    }

    Ok(())
}

unsafe fn modal_dialog_handles_escape(dialog: HWND, message: &MSG) -> bool {
    if message.message != WM_KEYDOWN || message.wParam != VK_ESCAPE_KEY || message.hwnd.is_null() {
        return false;
    }

    if message.hwnd == dialog || IsChild(dialog, message.hwnd) != 0 {
        DestroyWindow(dialog);
        return true;
    }

    false
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
    fn win32_about_dialog_uses_native_scroll_body_and_project_url_button() {
        let source = include_str!("dispatch.rs");
        assert!(source.contains("const ABOUT_DIALOG_WIDTH: i32 = 540;"));
        assert!(source.contains("const ABOUT_DIALOG_HEIGHT: i32 = 320;"));

        let show_body = compact(rust_function_body(source, "show_about_dialog"));
        for snippet in [
            "let title = app_about_title();",
            "let version_label = app_version_label();",
            "let content = text.about_message(env!(\"CARGO_PKG_VERSION\"));",
            "let project_url = utf8_to_wide_null(\"convert about project URL\", APP_AUTHOR_URL)?;",
            "show_about_modal_dialog(",
        ] {
            assert!(
                show_body.contains(&compact(snippet)),
                "about dialog show path should contain `{snippet}`"
            );
        }

        let controls_body = compact(rust_function_body(source, "create_about_dialog_controls"));
        for snippet in [
            "STATIC_CONTROL_CLASS_NAME",
            "version_label.as_ptr()",
            "dialog_text_control_style()",
            "configure_dialog_text_control(about_text, content)?;",
            "project_url.as_ptr()",
            "ABOUT_LINK_CONTROL_ID as HMENU",
            "ABOUT_OK_CONTROL_ID as HMENU",
        ] {
            assert!(
                controls_body.contains(&compact(snippet)),
                "about dialog controls should contain `{snippet}`"
            );
        }

        let proc_body = compact(rust_function_body(source, "about_dialog_proc"));
        for snippet in [
            "ABOUT_LINK_CONTROL_ID => {",
            "open_about_project_url(hwnd)",
            "show_app_error(hwnd, &error);",
            "ABOUT_OK_CONTROL_ID => {",
            "DestroyWindow(hwnd);",
        ] {
            assert!(
                proc_body.contains(&compact(snippet)),
                "about dialog proc should contain `{snippet}`"
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
