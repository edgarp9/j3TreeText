mod dispatch;
mod find_replace;
mod save_tabs;
mod search;
mod settings;
mod text_io;
mod tree_commands;

pub(super) use dispatch::{handle_command, select_all_editor_text};
pub(super) use find_replace::{
    active_replace_dialog, find_replace_message_id, handle_find_replace_message,
    open_find_dialog_from_window, open_replace_dialog_from_window,
};
pub(super) use save_tabs::{
    autosave_active_tab_before_navigation, close_active_tab_from_window,
    close_tab_at_index_from_window, handle_close, handle_tab_selection_changed,
    handle_tab_selection_changing, move_tab_from_window, resolve_dirty_before_refresh,
    save_current_document_from_window,
};
pub(super) use search::{
    handle_timer, pause_search_debounce_for_size_move, run_deferred_search_debounce_after_size_move,
};
pub(super) use tree_commands::{
    create_child_document_from_selection, create_sibling_document_from_selection,
    delete_selected_node_from_keyboard, move_selected_node_within_parent, rename_selected_node,
};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW;

use super::common::last_win32_error;
use super::i18n::ui_text;
use super::state::{TreeMode, WindowState};
use super::tabs::refresh_tab_control;
use super::text::utf8_to_wide_null;
use super::tree::search_is_active;
use crate::error::AppError;

unsafe fn refresh_tabs_for_state(state: &mut WindowState) -> Result<(), AppError> {
    refresh_tab_control(state.tab_bar, &state.tabs, &mut state.suppress_tab_change)
}

pub(super) unsafe fn update_window_title(hwnd: HWND, state: &WindowState) -> Result<(), AppError> {
    let text = ui_text(state.app.ui_settings().language);
    let mut title = state.app.window_title().to_owned();
    if state.tree_mode == TreeMode::Trash {
        title.push_str(" - ");
        title.push_str(text.window_trash_suffix());
    } else if search_is_active(state) {
        title.push_str(" - ");
        title.push_str(text.window_search_suffix());
    }
    if let Some(tab) = state.tabs.active() {
        title.push_str(" - ");
        title.push_str(&tab.display_title());
    }
    let title = utf8_to_wide_null("convert window title", &title)?;
    if SetWindowTextW(hwnd, title.as_ptr()) == 0 {
        return Err(last_win32_error("set window title"));
    }
    Ok(())
}
