use windows_sys::Win32::Foundation::{HWND, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use super::super::common::last_win32_error;
use super::super::layout::preferred_node_after_search_change;
use super::super::state::{TreeMode, WindowState};
use super::super::text::window_text_utf8;
use super::super::tree::{
    can_refine_search_from_visible_results, refresh_tree_after_search_change,
    refresh_tree_preserving_open_tab_content, search_is_active,
};
use super::super::window::{show_app_error, window_state};
use crate::error::AppError;

const SEARCH_DEBOUNCE_TIMER_ID: usize = 0x5345_0001;
const SEARCH_DEBOUNCE_DELAY_MS: u32 = 180;

pub(in crate::platform::win32) unsafe fn handle_search_changed(hwnd: HWND) -> Result<(), AppError> {
    let Some(mut state) = window_state(hwnd) else {
        return Ok(());
    };
    if state.suppress_search_change.get() {
        return Ok(());
    }

    let next_query = window_text_utf8(state.search, "search")?;
    handle_search_text_change(hwnd, &mut state, next_query, true)
}

pub(in crate::platform::win32) unsafe fn handle_timer(hwnd: HWND, wparam: WPARAM) -> bool {
    if wparam != SEARCH_DEBOUNCE_TIMER_ID {
        return false;
    }

    if let Err(error) = handle_search_debounce_timer(hwnd) {
        show_app_error(hwnd, &error);
    }
    true
}

unsafe fn handle_search_debounce_timer(hwnd: HWND) -> Result<(), AppError> {
    let Some(mut state) = window_state(hwnd) else {
        return Err(AppError::platform(
            "handle search debounce",
            "window state was not attached",
        ));
    };
    cancel_search_debounce_timer(hwnd, &mut state)?;
    if state.size_move.in_loop() {
        state.size_move.defer_search_debounce();
        return Ok(());
    }

    let next_query = window_text_utf8(state.search, "search")?;
    handle_search_text_change(hwnd, &mut state, next_query, false)
}

pub(in crate::platform::win32) unsafe fn pause_search_debounce_for_size_move(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<(), AppError> {
    if !state.search_debounce_timer_active {
        return Ok(());
    }

    cancel_search_debounce_timer(hwnd, state)?;
    state.size_move.defer_search_debounce();
    Ok(())
}

pub(in crate::platform::win32) unsafe fn run_deferred_search_debounce_after_size_move(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<(), AppError> {
    let next_query = window_text_utf8(state.search, "search")?;
    handle_search_text_change(hwnd, state, next_query, false)
}

unsafe fn handle_search_text_change(
    hwnd: HWND,
    state: &mut WindowState,
    next_query: String,
    allow_debounce: bool,
) -> Result<(), AppError> {
    if next_query == state.search_query {
        cancel_search_debounce_timer(hwnd, state)?;
        return Ok(());
    }
    let previous_query = state.search_query.clone();
    if next_query.trim() == previous_query.trim() {
        cancel_search_debounce_timer(hwnd, state)?;
        state.search_query = next_query;
        return Ok(());
    }

    if allow_debounce && should_debounce_search_change(state, &previous_query, &next_query) {
        restart_search_debounce_timer(hwnd, state)?;
        return Ok(());
    }

    cancel_search_debounce_timer(hwnd, state)?;
    apply_search_text_change_now(hwnd, state, next_query, previous_query)
}

fn should_debounce_search_change(
    state: &WindowState,
    previous_query: &str,
    next_query: &str,
) -> bool {
    state.tree_mode == TreeMode::Active
        && !next_query.trim().is_empty()
        && !can_refine_search_from_visible_results(state, previous_query, next_query)
}

unsafe fn apply_search_text_change_now(
    hwnd: HWND,
    state: &mut WindowState,
    next_query: String,
    previous_query: String,
) -> Result<(), AppError> {
    state.clear_current_find_match_highlight()?;
    state.store_editor_content_in_active_tab()?;
    let preferred_node_id = preferred_node_after_search_change(
        state.selected_node_id,
        state.app.ui_settings().selection,
    );
    state.tree_mode = TreeMode::Active;
    state.search_query = next_query;
    if !search_is_active(state) {
        state.app.reload_document()?;
        if state.sync_tabs_from_active_document_local_metadata(true)? {
            state.show_active_tab_in_editor()?;
        }
        return refresh_tree_preserving_open_tab_content(hwnd, state, preferred_node_id);
    }
    refresh_tree_after_search_change(hwnd, state, &previous_query, preferred_node_id)
}

unsafe fn restart_search_debounce_timer(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<(), AppError> {
    if SetTimer(
        hwnd,
        SEARCH_DEBOUNCE_TIMER_ID,
        SEARCH_DEBOUNCE_DELAY_MS,
        None,
    ) == 0
    {
        return Err(last_win32_error("set search debounce timer"));
    }
    state.search_debounce_timer_active = true;
    Ok(())
}

unsafe fn cancel_search_debounce_timer(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<(), AppError> {
    if !state.search_debounce_timer_active {
        return Ok(());
    }
    if KillTimer(hwnd, SEARCH_DEBOUNCE_TIMER_ID) == 0 {
        return Err(last_win32_error("cancel search debounce timer"));
    }
    state.search_debounce_timer_active = false;
    Ok(())
}
