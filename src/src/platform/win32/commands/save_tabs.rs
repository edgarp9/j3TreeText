use std::collections::HashSet;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, MessageBoxW, IDCANCEL, IDNO, IDYES, MB_ICONQUESTION, MB_YESNO, MB_YESNOCANCEL,
};

use super::super::i18n::ui_text;
use super::super::menu::update_menu_state;
use super::super::state::{UiDocument, WindowState};
use super::super::tabs::selected_tab_index;
use super::super::text::utf8_to_wide_null;
use super::super::tree::{refresh_tree, select_visible_tree_node_by_id};
use super::super::window::window_state;
use super::{refresh_tabs_for_state, update_window_title};
use crate::domain::{
    DirtyTabDecision, DocumentTabSource, DomainError, OpenDocumentTabInput, OpenTabs, UiLanguage,
    ROOT_NODE_ID,
};
use crate::error::AppError;

struct SaveContext {
    node_id: i64,
    parent_id: Option<i64>,
    title: String,
    expected_updated_at: String,
    save_target: bool,
}

pub(in crate::platform::win32) enum SaveOutcome {
    NoChanges,
    Saved,
    Reloaded,
    SavedAsNewDocument,
    Canceled,
}

enum ConflictDecision {
    Reload,
    SaveAsNewDocument,
    Cancel,
}

pub(in crate::platform::win32) unsafe fn save_current_document(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<SaveOutcome, AppError> {
    state.store_editor_content_in_active_tab()?;
    save_active_tab_content(hwnd, state)
}

unsafe fn save_active_tab_content(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<SaveOutcome, AppError> {
    let context = current_document_save_context(state)?;
    if !context.save_target {
        return Ok(SaveOutcome::NoChanges);
    }

    let save_result = {
        let content = active_tab_content(&state.tabs)?;
        state
            .app
            .save_document_content(context.node_id, content, &context.expected_updated_at)
    };

    let updated_at = match save_result {
        Ok(updated_at) => updated_at,
        Err(AppError::Domain(DomainError::DocumentSaveConflict { .. })) => {
            let content = active_tab_content(&state.tabs)?.to_owned();
            return handle_save_conflict(hwnd, state, context, content);
        }
        Err(error) => return Err(error),
    };

    update_loaded_ui_document_updated_at(&mut state.document, context.node_id, &updated_at);
    state.tabs.mark_active_current_content_saved(updated_at);
    refresh_tabs_for_state(state)?;
    update_window_title(hwnd, state)?;
    Ok(SaveOutcome::Saved)
}

fn save_outcome_allows_navigation(outcome: SaveOutcome) -> bool {
    matches!(
        outcome,
        SaveOutcome::NoChanges
            | SaveOutcome::Saved
            | SaveOutcome::Reloaded
            | SaveOutcome::SavedAsNewDocument
    )
}

pub(in crate::platform::win32) unsafe fn autosave_active_tab_before_navigation(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<bool, AppError> {
    state.store_editor_content_in_active_tab()?;
    let Some(tab) = state.tabs.active() else {
        return Ok(true);
    };
    if !tab.dirty {
        return Ok(true);
    }
    if !tab.is_save_target() {
        return resolve_unsavable_dirty_active_tab(hwnd, state);
    }

    Ok(save_outcome_allows_navigation(save_current_document(
        hwnd, state,
    )?))
}

unsafe fn autosave_all_dirty_tabs(hwnd: HWND, state: &mut WindowState) -> Result<bool, AppError> {
    state.store_editor_content_in_active_tab()?;
    while let Some(index) = state.tabs.first_dirty_index() {
        state.tabs.set_active(index);
        refresh_tabs_for_state(state)?;
        update_window_title(hwnd, state)?;

        let outcome = autosave_active_tab_from_memory_before_close(hwnd, state)?;
        if !save_outcome_allows_navigation(outcome) {
            state.show_active_tab_in_editor()?;
            refresh_tabs_for_state(state)?;
            update_window_title(hwnd, state)?;
            return Ok(false);
        }
    }

    Ok(true)
}

unsafe fn autosave_active_tab_from_memory_before_close(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<SaveOutcome, AppError> {
    let Some(tab) = state.tabs.active() else {
        return Ok(SaveOutcome::NoChanges);
    };
    if !tab.dirty {
        return Ok(SaveOutcome::NoChanges);
    }
    if !tab.is_save_target() {
        return if resolve_unsavable_dirty_active_tab_for_close(hwnd, state)? {
            Ok(SaveOutcome::NoChanges)
        } else {
            Ok(SaveOutcome::Canceled)
        };
    }

    save_active_tab_content(hwnd, state)
}

unsafe fn resolve_unsavable_dirty_active_tab(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<bool, AppError> {
    resolve_unsavable_dirty_active_tab_with_editor_refresh(hwnd, state, true)
}

unsafe fn resolve_unsavable_dirty_active_tab_for_close(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<bool, AppError> {
    resolve_unsavable_dirty_active_tab_with_editor_refresh(hwnd, state, false)
}

unsafe fn resolve_unsavable_dirty_active_tab_with_editor_refresh(
    hwnd: HWND,
    state: &mut WindowState,
    refresh_editor: bool,
) -> Result<bool, AppError> {
    let tab_title = state.tabs.active().map(|tab| tab.title.clone());
    if !prompt_discard_unsavable_changes(
        hwnd,
        tab_title.as_deref(),
        state.app.ui_settings().language,
    )? {
        return Ok(false);
    }

    if let Some(index) = state.tabs.active_index() {
        state.tabs.discard_tab_changes(index);
        if refresh_editor {
            state.show_active_tab_in_editor()?;
        }
        refresh_tabs_for_state(state)?;
        update_window_title(hwnd, state)?;
    }
    Ok(true)
}

fn update_loaded_ui_document_updated_at(document: &mut UiDocument, node_id: i64, updated_at: &str) {
    if let Some(node) = document.nodes.iter_mut().find(|node| node.id == node_id) {
        node.updated_at = updated_at.to_owned();
    }
}

fn active_tab_content(tabs: &OpenTabs) -> Result<&str, AppError> {
    let Some(tab) = tabs.active() else {
        return Err(DomainError::NodeNotFound { node_id: 0 }.into());
    };

    Ok(tab.content.as_str())
}

fn current_document_save_context(state: &WindowState) -> Result<SaveContext, AppError> {
    let Some(tab) = state.tabs.active() else {
        return Err(DomainError::NodeNotFound { node_id: 0 }.into());
    };

    Ok(SaveContext {
        node_id: tab.node_id,
        parent_id: tab.parent_id,
        title: tab.title.clone(),
        expected_updated_at: tab.loaded_updated_at.clone(),
        save_target: tab.is_save_target(),
    })
}

unsafe fn handle_save_conflict(
    hwnd: HWND,
    state: &mut WindowState,
    context: SaveContext,
    content: String,
) -> Result<SaveOutcome, AppError> {
    let language = state.app.ui_settings().language;
    match prompt_save_conflict(hwnd, language)? {
        ConflictDecision::Reload => {
            state.app.reload_document()?;
            state.sync_tabs_from_active_document_metadata(false)?;
            if let Some(input) = tab_input_from_active_document(state, context.node_id)? {
                state.tabs.reload_active(input);
                state.show_active_tab_in_editor()?;
            }
            refresh_tree(hwnd, state, Some(context.node_id))?;
            restore_active_tab_after_refresh(hwnd, state, context.node_id)?;
            Ok(SaveOutcome::Reloaded)
        }
        ConflictDecision::SaveAsNewDocument => {
            let parent_id = context.parent_id.unwrap_or(ROOT_NODE_ID);
            let base_title = conflicted_copy_title(&context.title, language);
            let node_id = state.app.save_document_content_as_new_document(
                parent_id,
                &base_title,
                &content,
            )?;
            if let Some(input) = tab_input_from_active_document(state, node_id)? {
                state.tabs.replace_active(input);
                state.show_active_tab_in_editor()?;
            }
            refresh_tree(hwnd, state, Some(node_id))?;
            restore_active_tab_after_refresh(hwnd, state, node_id)?;
            Ok(SaveOutcome::SavedAsNewDocument)
        }
        ConflictDecision::Cancel => Ok(SaveOutcome::Canceled),
    }
}

unsafe fn prompt_save_conflict(
    hwnd: HWND,
    language: UiLanguage,
) -> Result<ConflictDecision, AppError> {
    let message = ui_text(language).save_conflict_message();
    let title = utf8_to_wide_null("convert save conflict title", "j3TreeText")?;
    let message = utf8_to_wide_null("convert save conflict message", message)?;
    let result = MessageBoxW(
        hwnd,
        message.as_ptr(),
        title.as_ptr(),
        MB_YESNOCANCEL | MB_ICONQUESTION,
    );

    Ok(match result {
        IDYES => ConflictDecision::Reload,
        IDNO => ConflictDecision::SaveAsNewDocument,
        IDCANCEL => ConflictDecision::Cancel,
        _ => ConflictDecision::Cancel,
    })
}

fn conflicted_copy_title(title: &str, language: UiLanguage) -> String {
    ui_text(language).conflicted_copy_title(title)
}

fn tab_input_from_active_document(
    state: &WindowState,
    node_id: i64,
) -> Result<Option<OpenDocumentTabInput>, AppError> {
    let Some((node_id, parent_id, title, editable)) = state
        .app
        .document()
        .nodes()
        .iter()
        .find(|node| node.id == node_id)
        .map(|node| {
            (
                node.id,
                node.parent_id,
                node.title.clone(),
                node.deleted_at.is_none(),
            )
        })
    else {
        return Ok(None);
    };
    let (content, updated_at) = state.app.load_active_node_content(node_id)?;
    Ok(Some(OpenDocumentTabInput {
        node_id,
        parent_id,
        title,
        content,
        loaded_updated_at: updated_at,
        editable,
        source: DocumentTabSource::ActiveTree,
    }))
}

unsafe fn restore_active_tab_after_refresh(
    hwnd: HWND,
    state: &mut WindowState,
    node_id: i64,
) -> Result<(), AppError> {
    if !state.tabs.set_active_by_node_id(node_id) {
        return Ok(());
    }

    state.show_active_tab_in_editor()?;
    refresh_tabs_for_state(state)?;
    select_visible_tree_node_by_id(state, node_id)?;
    update_menu_state(hwnd, state)?;
    update_window_title(hwnd, state)
}

pub(in crate::platform::win32) unsafe fn save_current_document_from_window(
    hwnd: HWND,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform("save document", "window state was not attached"))?;
    if state.tabs.active().is_none() {
        return Ok(());
    }
    save_current_document(hwnd, &mut state).map(|_| ())
}

pub(in crate::platform::win32) unsafe fn close_active_tab_from_window(
    hwnd: HWND,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform("close tab", "window state was not attached"))?;
    close_active_tab(hwnd, &mut state)
}

pub(in crate::platform::win32) unsafe fn close_tab_at_index_from_window(
    hwnd: HWND,
    index: usize,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform("close tab", "window state was not attached"))?;
    let Some(active_index) = state.tabs.active_index() else {
        state.show_active_tab_in_editor()?;
        return Ok(());
    };
    if index >= state.tabs.tabs().len() {
        return Ok(());
    }

    if active_index != index {
        if !autosave_active_tab_before_navigation(hwnd, &mut state)? {
            return Ok(());
        }
        if !state.tabs.set_active(index) {
            return Ok(());
        }
        show_selected_tab(hwnd, &mut state)?;
    }

    close_active_tab(hwnd, &mut state)
}

pub(in crate::platform::win32) unsafe fn move_tab_from_window(
    hwnd: HWND,
    from_index: usize,
    to_index: usize,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform("move tab", "window state was not attached"))?;
    state.store_editor_content_in_active_tab()?;
    if !state.tabs.move_tab(from_index, to_index) {
        return Ok(());
    }

    refresh_tabs_for_state(&mut state)?;
    update_window_title(hwnd, &state)
}

unsafe fn close_active_tab(hwnd: HWND, state: &mut WindowState) -> Result<(), AppError> {
    if state.tabs.active().is_none() {
        state.show_active_tab_in_editor()?;
        return Ok(());
    }
    if !autosave_active_tab_before_navigation(hwnd, state)? {
        return Ok(());
    }

    state.tabs.close_active();
    state.show_active_tab_in_editor()?;
    refresh_tabs_for_state(state)?;
    if let Some(node_id) = state.tabs.active().map(|tab| tab.node_id) {
        select_visible_tree_node_by_id(state, node_id)?;
    }
    update_menu_state(hwnd, state)?;
    update_window_title(hwnd, state)
}

unsafe fn show_selected_tab(hwnd: HWND, state: &mut WindowState) -> Result<(), AppError> {
    state.show_active_tab_in_editor()?;
    refresh_tabs_for_state(state)?;
    if let Some(node_id) = state.tabs.active().map(|tab| tab.node_id) {
        select_visible_tree_node_by_id(state, node_id)?;
    }
    update_menu_state(hwnd, state)?;
    update_window_title(hwnd, state)
}

pub(in crate::platform::win32) unsafe fn handle_tab_selection_changed(
    hwnd: HWND,
) -> Result<(), AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("handle tab selection", "window state was not attached")
    })?;
    if state.suppress_tab_change {
        return Ok(());
    }

    let Some(index) = selected_tab_index(state.tab_bar) else {
        return Ok(());
    };
    state.store_editor_content_in_active_tab()?;
    if !state.tabs.set_active(index) {
        return Ok(());
    }
    state.show_active_tab_in_editor()?;
    refresh_tabs_for_state(&mut state)?;
    if let Some(node_id) = state.tabs.active().map(|tab| tab.node_id) {
        select_visible_tree_node_by_id(&mut state, node_id)?;
    }
    update_menu_state(hwnd, &state)?;
    update_window_title(hwnd, &state)
}

pub(in crate::platform::win32) unsafe fn handle_tab_selection_changing(
    hwnd: HWND,
) -> Result<bool, AppError> {
    let mut state = window_state(hwnd).ok_or_else(|| {
        AppError::platform("handle tab selection", "window state was not attached")
    })?;
    if state.suppress_tab_change {
        return Ok(true);
    }

    autosave_active_tab_before_navigation(hwnd, &mut state)
}

pub(in crate::platform::win32) unsafe fn resolve_dirty_tabs_for_nodes(
    hwnd: HWND,
    state: &mut WindowState,
    node_ids: &HashSet<i64>,
) -> Result<bool, AppError> {
    state.store_editor_content_in_active_tab()?;

    loop {
        let dirty_indices = state.tabs.dirty_tab_indices_for_node_set(node_ids);
        let Some(index) = dirty_indices.first().copied() else {
            return Ok(true);
        };

        state.tabs.set_active(index);
        state.show_active_tab_in_editor()?;
        refresh_tabs_for_state(state)?;
        update_window_title(hwnd, state)?;

        let Some((has_unsavable_changes, tab_title)) = state
            .tabs
            .active()
            .map(|tab| (tab.has_unsavable_changes(), tab.title.clone()))
        else {
            return Ok(false);
        };

        if has_unsavable_changes {
            if !resolve_unsavable_dirty_active_tab(hwnd, state)? {
                return Ok(false);
            }
            continue;
        }

        match prompt_unsaved_changes(hwnd, Some(&tab_title), state.app.ui_settings().language)? {
            DirtyTabDecision::Save => match save_current_document(hwnd, state)? {
                SaveOutcome::Saved | SaveOutcome::NoChanges => {}
                SaveOutcome::Reloaded | SaveOutcome::SavedAsNewDocument | SaveOutcome::Canceled => {
                    return Ok(false)
                }
            },
            DirtyTabDecision::Discard => {
                state.tabs.discard_tab_changes(index);
                state.show_active_tab_in_editor()?;
                refresh_tabs_for_state(state)?;
            }
            DirtyTabDecision::Cancel => return Ok(false),
        }
    }
}

pub(in crate::platform::win32) unsafe fn resolve_dirty_before_refresh(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<bool, AppError> {
    state.store_editor_content_in_active_tab()?;
    state.sync_tabs_from_visible_document(false)?;
    refresh_tabs_for_state(state)?;
    update_window_title(hwnd, state)?;
    Ok(true)
}

pub(in crate::platform::win32) unsafe fn prompt_unsaved_changes(
    hwnd: HWND,
    title_text: Option<&str>,
    language: UiLanguage,
) -> Result<DirtyTabDecision, AppError> {
    let message = ui_text(language).unsaved_changes(title_text);
    let title = utf8_to_wide_null("convert unsaved changes title", "j3TreeText")?;
    let message = utf8_to_wide_null("convert unsaved changes message", &message)?;
    let result = MessageBoxW(
        hwnd,
        message.as_ptr(),
        title.as_ptr(),
        MB_YESNOCANCEL | MB_ICONQUESTION,
    );

    Ok(match result {
        IDYES => DirtyTabDecision::Save,
        IDNO => DirtyTabDecision::Discard,
        IDCANCEL => DirtyTabDecision::Cancel,
        _ => DirtyTabDecision::Cancel,
    })
}

unsafe fn prompt_discard_unsavable_changes(
    hwnd: HWND,
    title_text: Option<&str>,
    language: UiLanguage,
) -> Result<bool, AppError> {
    let message = ui_text(language).discard_unsavable_changes(title_text);
    let title = utf8_to_wide_null("convert unsavable changes title", "j3TreeText")?;
    let message = utf8_to_wide_null("convert unsavable changes message", &message)?;
    let result = MessageBoxW(
        hwnd,
        message.as_ptr(),
        title.as_ptr(),
        MB_YESNO | MB_ICONQUESTION,
    );

    Ok(result == IDYES)
}

pub(in crate::platform::win32) unsafe fn handle_close(hwnd: HWND) -> Result<bool, AppError> {
    let mut state = window_state(hwnd)
        .ok_or_else(|| AppError::platform("close window", "window state was not attached"))?;
    autosave_all_dirty_tabs(hwnd, &mut state)
}

pub(in crate::platform::win32) unsafe fn close_window_from_menu(
    hwnd: HWND,
) -> Result<(), AppError> {
    if handle_close(hwnd)? {
        DestroyWindow(hwnd);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::super::state::UiNode;
    use super::*;

    #[test]
    fn update_loaded_ui_document_updated_at_updates_matching_node() {
        let mut document = UiDocument::from_ui_nodes(vec![UiNode {
            id: 7,
            parent_id: Some(ROOT_NODE_ID),
            display_parent_id: Some(ROOT_NODE_ID),
            title: "Draft".to_owned(),
            sort_order: 0,
            title_sort_key: "Draft".to_owned(),
            display_title: Vec::new(),
            search_content_matched: false,
            updated_at: "2026-05-21T00:00:00Z".to_owned(),
            editable: true,
            source: DocumentTabSource::ActiveTree,
        }]);

        update_loaded_ui_document_updated_at(&mut document, 7, "2026-05-22T00:00:00Z");

        let Some(node) = document.node_by_id(7) else {
            panic!("updated node should exist");
        };
        assert_eq!(node.updated_at, "2026-05-22T00:00:00Z");
        assert_eq!(node.title, "Draft");
    }
}
