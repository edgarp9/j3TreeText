use std::collections::HashSet;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDYES, MB_ICONQUESTION, MB_YESNO};

use super::super::i18n::ui_text;
use super::super::menu::update_menu_state;
use super::super::state::{TreeMode, WindowState};
use super::super::text::{set_search_text, utf8_to_wide_null, ProgrammaticTextUpdateGuard};
use super::super::tree::{
    refresh_tree, refresh_tree_after_active_document_delete,
    refresh_tree_after_active_document_insert, refresh_tree_after_active_document_move,
    refresh_tree_preserving_open_tab_content, search_is_active, start_selected_label_edit,
    subtree_node_ids,
};
use super::super::window::window_state;
use super::refresh_tabs_for_state;
use super::save_tabs::{resolve_dirty_before_refresh, resolve_dirty_tabs_for_nodes};
use crate::domain::{DomainError, SiblingMoveDirection, UiLanguage, ROOT_NODE_ID};
use crate::error::AppError;

pub(in crate::platform::win32) unsafe fn create_sibling_document_from_selection(
    hwnd: HWND,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("handle node command", "window state was not attached")
    })?;
    ensure_active_tree_browse_mode(&state)?;
    if !resolve_dirty_before_refresh(hwnd, &mut state)? {
        update_menu_state(hwnd, &state)?;
        return Ok(());
    }

    let parent_id = selected_sibling_parent_id(&state)?;
    let node_id = state.app.create_document(parent_id)?;

    refresh_tree_after_active_document_insert(hwnd, &mut state, node_id)?;
    start_selected_label_edit(&state)
}

pub(in crate::platform::win32) unsafe fn create_child_document_from_selection(
    hwnd: HWND,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("handle child node command", "window state was not attached")
    })?;
    ensure_active_tree_browse_mode(&state)?;
    if !resolve_dirty_before_refresh(hwnd, &mut state)? {
        update_menu_state(hwnd, &state)?;
        return Ok(());
    }

    let parent_id = selected_child_parent_id(&state)?;
    let node_id = state.app.create_document(parent_id)?;

    refresh_tree_after_active_document_insert(hwnd, &mut state, node_id)?;
    start_selected_label_edit(&state)
}

pub(in crate::platform::win32) unsafe fn rename_selected_node(hwnd: HWND) -> Result<(), AppError> {
    let state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("handle rename command", "window state was not attached")
    })?;
    ensure_active_tree_browse_mode(&state)?;
    ensure_selected_node(&state)?;
    start_selected_label_edit(&state)
}

pub(in crate::platform::win32) unsafe fn delete_selected_node(hwnd: HWND) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("handle delete command", "window state was not attached")
    })?;
    ensure_active_tree_browse_mode(&state)?;

    let Some(node) = state.selected_node() else {
        return Err(DomainError::NodeNotFound { node_id: 0 }.into());
    };
    let node_id = node.id;
    let parent_id = node.parent_id;
    let title = node.title.clone();
    ensure_selected_node_can_be_deleted(node_id, parent_id)?;

    let affected_node_ids = subtree_node_ids(&state.document, node_id);

    if !confirm_delete(hwnd, &title, state.app.ui_settings().language)? {
        return Ok(());
    }

    let affected_node_id_set = affected_node_ids.iter().copied().collect::<HashSet<_>>();
    if !resolve_dirty_tabs_for_nodes(hwnd, &mut state, &affected_node_id_set)? {
        update_menu_state(hwnd, &state)?;
        return Ok(());
    }

    state.app.delete_node(node_id)?;
    state.tabs.close_tabs_for_node_set(&affected_node_id_set);
    state.sync_tabs_from_active_document_local_metadata(true)?;
    state.show_active_tab_in_editor()?;
    refresh_tabs_for_state(&mut state)?;
    refresh_tree_after_active_document_delete(hwnd, &mut state, &affected_node_ids, parent_id)
}

pub(in crate::platform::win32) unsafe fn delete_selected_node_from_keyboard(
    hwnd: HWND,
) -> Result<(), AppError> {
    let (tree_mode, tree) = {
        let state = window_state(hwnd).ok_or_else(|| {
            AppError::platform("handle delete shortcut", "window state was not attached")
        })?;
        (state.tree_mode, state.tree)
    };

    let result = match tree_mode {
        TreeMode::Active => delete_selected_node(hwnd),
        TreeMode::Trash => permanently_delete_selected_node(hwnd),
    };
    SetFocus(tree);
    result
}

pub(in crate::platform::win32) unsafe fn restore_selected_node(hwnd: HWND) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("handle restore command", "window state was not attached")
    })?;
    ensure_trash_tree_mode(&state)?;

    let Some(node) = state.selected_node() else {
        return Err(DomainError::NodeNotDeleted { node_id: 0 }.into());
    };
    let node_id = node.id;

    if !confirm_restore(hwnd, &node.title, state.app.ui_settings().language)? {
        return Ok(());
    }

    state.app.restore_node(node_id)?;
    state.sync_tabs_from_active_document_local_metadata(true)?;
    state.show_active_tab_in_editor()?;
    refresh_tabs_for_state(&mut state)?;
    refresh_tree_preserving_open_tab_content(hwnd, &mut state, None)
}

pub(in crate::platform::win32) unsafe fn permanently_delete_selected_node(
    hwnd: HWND,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform(
            "handle permanent delete command",
            "window state was not attached",
        )
    })?;
    ensure_trash_tree_mode(&state)?;

    let Some(node) = state.selected_node() else {
        return Err(DomainError::NodeNotDeleted { node_id: 0 }.into());
    };
    let node_id = node.id;
    let parent_id = node.parent_id;
    let title = node.title.clone();
    ensure_selected_node_can_be_deleted(node_id, parent_id)?;

    let affected_node_ids = subtree_node_ids(&state.document, node_id);

    if !confirm_permanent_delete(hwnd, &title, state.app.ui_settings().language)? {
        return Ok(());
    }

    let affected_node_id_set = affected_node_ids.iter().copied().collect::<HashSet<_>>();
    if !resolve_dirty_tabs_for_nodes(hwnd, &mut state, &affected_node_id_set)? {
        update_menu_state(hwnd, &state)?;
        return Ok(());
    }

    state.app.permanently_delete_node(node_id)?;
    state.tabs.close_tabs_for_node_set(&affected_node_id_set);
    state.show_active_tab_in_editor()?;
    refresh_tabs_for_state(&mut state)?;
    refresh_tree_preserving_open_tab_content(hwnd, &mut state, None)
}

pub(in crate::platform::win32) unsafe fn show_tree_mode(
    hwnd: HWND,
    tree_mode: TreeMode,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform("switch tree mode", "window state was not attached"))?;
    if state.tree_mode == tree_mode && !(tree_mode == TreeMode::Active && search_is_active(&state))
    {
        update_menu_state(hwnd, &state)?;
        return Ok(());
    }
    if !resolve_dirty_before_refresh(hwnd, &mut state)? {
        update_menu_state(hwnd, &state)?;
        return Ok(());
    }

    state.tree_mode = tree_mode;
    if !state.search_query.is_empty() {
        state.search_query.clear();
        {
            let _guard = ProgrammaticTextUpdateGuard::enter(&state.suppress_search_change);
            set_search_text(state.search, "")?;
        }
    }
    if state.tree_mode == TreeMode::Active {
        state.app.reload_document()?;
        if state.sync_tabs_from_reloaded_active_document_metadata(true)? {
            state.show_active_tab_in_editor()?;
        }
        return refresh_tree_preserving_open_tab_content(hwnd, &mut state, None);
    }
    refresh_tree(hwnd, &mut state, None)
}

pub(in crate::platform::win32) unsafe fn move_selected_node_within_parent(
    hwnd: HWND,
    direction: SiblingMoveDirection,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("handle reorder command", "window state was not attached")
    })?;
    ensure_active_tree_browse_mode(&state)?;
    if !resolve_dirty_before_refresh(hwnd, &mut state)? {
        update_menu_state(hwnd, &state)?;
        return Ok(());
    }

    let Some(node) = state.selected_node() else {
        return Err(DomainError::NodeNotFound { node_id: 0 }.into());
    };
    let node_id = node.id;
    let parent_id = node.parent_id;

    state.app.move_node_within_parent(node_id, direction)?;
    state.sync_tabs_from_active_document_local_metadata(true)?;
    refresh_tabs_for_state(&mut state)?;
    refresh_tree_after_active_document_move(hwnd, &mut state, node_id, parent_id, parent_id)
}

fn selected_sibling_parent_id(state: &WindowState) -> Result<i64, AppError> {
    let Some(node) = state.selected_node() else {
        return Err(DomainError::NodeNotFound { node_id: 0 }.into());
    };

    if node.id == ROOT_NODE_ID {
        return Ok(ROOT_NODE_ID);
    }

    node.parent_id
        .ok_or(DomainError::NodeNotFound { node_id: node.id }.into())
}

fn selected_child_parent_id(state: &WindowState) -> Result<i64, AppError> {
    let Some(node) = state.selected_node() else {
        return Err(DomainError::NodeNotFound { node_id: 0 }.into());
    };

    Ok(node.id)
}

fn ensure_selected_node(state: &WindowState) -> Result<(), AppError> {
    state
        .selected_node()
        .map(|_| ())
        .ok_or_else(|| DomainError::NodeNotFound { node_id: 0 }.into())
}

fn ensure_selected_node_can_be_deleted(
    node_id: i64,
    parent_id: Option<i64>,
) -> Result<(), AppError> {
    if node_id == ROOT_NODE_ID || parent_id.is_none() {
        return Err(DomainError::CannotDeleteRoot.into());
    }

    Ok(())
}

fn ensure_active_tree_mode(state: &WindowState) -> Result<(), AppError> {
    if state.tree_mode == TreeMode::Active {
        Ok(())
    } else {
        Err(AppError::user(
            ui_text(state.app.ui_settings().language).active_tree_only(),
        ))
    }
}

fn ensure_active_tree_browse_mode(state: &WindowState) -> Result<(), AppError> {
    ensure_active_tree_mode(state)?;
    if search_is_active(state) {
        return Err(AppError::user(
            ui_text(state.app.ui_settings().language).search_not_allowed(),
        ));
    }

    Ok(())
}

fn ensure_trash_tree_mode(state: &WindowState) -> Result<(), AppError> {
    if state.tree_mode == TreeMode::Trash {
        Ok(())
    } else {
        Err(AppError::user(
            ui_text(state.app.ui_settings().language).trash_only(),
        ))
    }
}

unsafe fn confirm_delete(hwnd: HWND, title: &str, language: UiLanguage) -> Result<bool, AppError> {
    let message = ui_text(language).confirm_delete(title);
    let title = utf8_to_wide_null("convert delete confirmation title", "j3TreeText")?;
    let message = utf8_to_wide_null("convert delete confirmation message", &message)?;
    let result = MessageBoxW(
        hwnd,
        message.as_ptr(),
        title.as_ptr(),
        MB_YESNO | MB_ICONQUESTION,
    );

    Ok(result == IDYES)
}

unsafe fn confirm_restore(hwnd: HWND, title: &str, language: UiLanguage) -> Result<bool, AppError> {
    let message = ui_text(language).confirm_restore(title);
    let title = utf8_to_wide_null("convert restore confirmation title", "j3TreeText")?;
    let message = utf8_to_wide_null("convert restore confirmation message", &message)?;
    let result = MessageBoxW(
        hwnd,
        message.as_ptr(),
        title.as_ptr(),
        MB_YESNO | MB_ICONQUESTION,
    );

    Ok(result == IDYES)
}

unsafe fn confirm_permanent_delete(
    hwnd: HWND,
    title: &str,
    language: UiLanguage,
) -> Result<bool, AppError> {
    let message = ui_text(language).confirm_permanent_delete(title);
    let title = utf8_to_wide_null("convert permanent delete confirmation title", "j3TreeText")?;
    let message = utf8_to_wide_null("convert permanent delete confirmation message", &message)?;
    let result = MessageBoxW(
        hwnd,
        message.as_ptr(),
        title.as_ptr(),
        MB_YESNO | MB_ICONQUESTION,
    );

    Ok(result == IDYES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_node_is_rejected_before_delete_side_effects() {
        let error = match ensure_selected_node_can_be_deleted(ROOT_NODE_ID, None) {
            Ok(()) => panic!("root node should not be deletable"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::CannotDeleteRoot)
        ));
    }

    #[test]
    fn nonstandard_root_node_is_rejected_before_delete_side_effects() {
        let error = match ensure_selected_node_can_be_deleted(ROOT_NODE_ID + 100, None) {
            Ok(()) => panic!("node without parent should not be deletable"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::CannotDeleteRoot)
        ));
    }
}
