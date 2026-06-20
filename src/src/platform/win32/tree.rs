use std::{
    collections::{HashMap, HashSet},
    ffi::c_void,
    mem, ptr,
};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
use windows_sys::Win32::UI::Controls::{
    HTREEITEM, NMHDR, NMTREEVIEWW, NMTVDISPINFOW, TVE_EXPAND, TVGN_CARET, TVGN_CHILD,
    TVGN_DROPHILITE, TVGN_NEXT, TVGN_ROOT, TVHITTESTINFO, TVIF_PARAM, TVIF_TEXT, TVINSERTSTRUCTW,
    TVINSERTSTRUCTW_0, TVITEMW, TVI_FIRST, TVI_LAST, TVI_ROOT, TVM_DELETEITEM, TVM_EDITLABELW,
    TVM_EXPAND, TVM_GETEDITCONTROL, TVM_GETITEMW, TVM_GETNEXTITEM, TVM_HITTEST, TVM_INSERTITEMW,
    TVM_SELECTITEM, TVM_SETITEMW, TVN_BEGINDRAGW, TVN_BEGINLABELEDITW, TVN_ENDLABELEDITW,
    TVN_SELCHANGEDW, TVN_SELCHANGINGW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SendMessageW, SetWindowTextW, WM_KEYDOWN, WM_KILLFOCUS, WM_NCDESTROY,
};

use super::commands::{
    autosave_active_tab_before_navigation, resolve_dirty_before_refresh, update_window_title,
};
use super::common::{
    last_win32_error, CONTROL_TREE_ID, VK_ESCAPE_KEY, WM_APP_REFRESH_TREE_AFTER_LABEL_EDIT,
};
use super::layout::{
    client_point_from_lparam, save_current_tree_refresh_ui_settings, save_current_ui_settings,
    tree_top_offset,
};
use super::menu::update_menu_state;
use super::state::{
    compare_ui_nodes_for_display, InitialSelection, TreeItemHandlesByNodeId, TreeLabelEditState,
    TreeMode, TreePopulation, UiDocument, UiNode, WindowState,
};
use super::tabs::refresh_tab_control;
use super::text::{utf8_to_wide_null_lossy, wide_null_to_string, window_text_utf8};
use super::window::{show_app_error, show_app_error_for_language, window_state};
use crate::domain::{DocumentTabSource, DomainError, SEARCH_RESULT_LIMIT};
use crate::error::AppError;

const MEM_COMMIT: u32 = 0x1000;
const PAGE_NOACCESS: u32 = 0x01;
const PAGE_READONLY: u32 = 0x02;
const PAGE_READWRITE: u32 = 0x04;
const PAGE_WRITECOPY: u32 = 0x08;
const PAGE_EXECUTE_READ: u32 = 0x20;
const PAGE_EXECUTE_READWRITE: u32 = 0x40;
const PAGE_EXECUTE_WRITECOPY: u32 = 0x80;
const PAGE_GUARD: u32 = 0x100;
const PAGE_READABLE_MASK: u32 = PAGE_READONLY
    | PAGE_READWRITE
    | PAGE_WRITECOPY
    | PAGE_EXECUTE_READ
    | PAGE_EXECUTE_READWRITE
    | PAGE_EXECUTE_WRITECOPY;
const END_LABEL_EDIT_REJECT_CONTROL_TEXT: LRESULT = 0;
const MIN_FTS_TRIGRAM_CHARS: usize = 3;
const TREE_LABEL_EDIT_SUBCLASS_ID: usize = 1;

type ChildIndicesByParent = HashMap<Option<i64>, Vec<usize>>;
type ChildIdsByParent = HashMap<i64, Vec<i64>>;

#[repr(C)]
#[allow(dead_code)]
struct MemoryBasicInformation {
    base_address: *mut c_void,
    allocation_base: *mut c_void,
    allocation_protect: u32,
    partition_id: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    type_: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn VirtualQuery(
        address: *const c_void,
        buffer: *mut MemoryBasicInformation,
        length: usize,
    ) -> usize;
}

pub(super) unsafe fn populate_tree(
    tree: HWND,
    document: &UiDocument,
    preferred_node_id: Option<i64>,
) -> Result<Option<InitialSelection>, AppError> {
    let initially_expanded_node_ids =
        preferred_display_ancestor_node_ids(document, preferred_node_id);
    populate_tree_with_expansion(
        tree,
        document,
        preferred_node_id,
        TreeExpansion::OnlyNodeIds(&initially_expanded_node_ids),
    )
    .map(|(selection, _)| selection)
}

unsafe fn populate_tree_with_item_handles(
    tree: HWND,
    document: &UiDocument,
    preferred_node_id: Option<i64>,
) -> Result<(Option<InitialSelection>, TreeItemHandlesByNodeId), AppError> {
    populate_tree_with_expansion(tree, document, preferred_node_id, TreeExpansion::All)
}

unsafe fn populate_tree_with_expansion(
    tree: HWND,
    document: &UiDocument,
    preferred_node_id: Option<i64>,
    expansion: TreeExpansion<'_>,
) -> Result<(Option<InitialSelection>, TreeItemHandlesByNodeId), AppError> {
    let mut population = TreePopulation::new(preferred_node_id, document.nodes.len());
    if document.child_indices_are_display_ordered() {
        insert_child_items_from_display_ordered_ranges(
            tree,
            document,
            None,
            TVI_ROOT,
            &mut population,
            expansion,
        )?;
    } else {
        let child_indices_by_parent = display_child_indices_by_parent(document);
        insert_child_items(
            tree,
            document,
            &child_indices_by_parent,
            None,
            TVI_ROOT,
            &mut population,
            expansion,
        )?;
    }
    Ok(population.into_parts())
}

#[derive(Clone, Copy)]
enum TreeExpansion<'a> {
    All,
    OnlyNodeIds(&'a HashSet<i64>),
}

impl TreeExpansion<'_> {
    fn should_expand(self, node_id: i64) -> bool {
        match self {
            Self::All => true,
            Self::OnlyNodeIds(node_ids) => node_ids.contains(&node_id),
        }
    }
}

fn preferred_display_ancestor_node_ids(
    document: &UiDocument,
    preferred_node_id: Option<i64>,
) -> HashSet<i64> {
    let mut ancestor_ids = HashSet::new();
    let mut current_parent_id = preferred_node_id
        .and_then(|node_id| document.node_index_by_id(node_id))
        .and_then(|node_index| document.nodes.get(node_index))
        .and_then(|node| node.display_parent_id);

    while let Some(parent_id) = current_parent_id {
        if !ancestor_ids.insert(parent_id) {
            break;
        }
        current_parent_id = document
            .node_index_by_id(parent_id)
            .and_then(|node_index| document.nodes.get(node_index))
            .and_then(|node| node.display_parent_id);
    }

    ancestor_ids
}

unsafe fn insert_child_items(
    tree: HWND,
    document: &UiDocument,
    child_indices_by_parent: &ChildIndicesByParent,
    parent_id: Option<i64>,
    parent_handle: HTREEITEM,
    population: &mut TreePopulation,
    expansion: TreeExpansion<'_>,
) -> Result<(), AppError> {
    enum InsertTask {
        InsertChildren {
            parent_id: Option<i64>,
            parent_handle: HTREEITEM,
        },
        InsertNode {
            parent_handle: HTREEITEM,
            node_index: usize,
        },
        ExpandFolder {
            handle: HTREEITEM,
        },
    }

    let mut tasks = vec![InsertTask::InsertChildren {
        parent_id,
        parent_handle,
    }];

    while let Some(task) = tasks.pop() {
        match task {
            InsertTask::InsertChildren {
                parent_id,
                parent_handle,
            } => {
                if let Some(children) = child_indices_by_parent.get(&parent_id) {
                    for &node_index in children.iter().rev() {
                        tasks.push(InsertTask::InsertNode {
                            parent_handle,
                            node_index,
                        });
                    }
                }
            }
            InsertTask::InsertNode {
                parent_handle,
                node_index,
            } => {
                let node = &document.nodes[node_index];
                let handle = insert_tree_item(tree, parent_handle, TVI_LAST, node)?;

                population.remember(node, node_index, handle);

                if expansion.should_expand(node.id) {
                    tasks.push(InsertTask::ExpandFolder { handle });
                }

                tasks.push(InsertTask::InsertChildren {
                    parent_id: Some(node.id),
                    parent_handle: handle,
                });
            }
            InsertTask::ExpandFolder { handle } => {
                SendMessageW(tree, TVM_EXPAND, TVE_EXPAND as WPARAM, handle as LPARAM);
            }
        }
    }

    Ok(())
}

unsafe fn insert_tree_item(
    tree: HWND,
    parent_handle: HTREEITEM,
    insert_after: HTREEITEM,
    node: &UiNode,
) -> Result<HTREEITEM, AppError> {
    let lparam = LPARAM::try_from(node.id)
        .map_err(|_| AppError::platform("insert tree item", "node id is invalid for TreeView"))?;
    let title = node.display_title.as_slice();
    let text_len = i32::try_from(title.len()).map_err(|_| {
        AppError::platform("insert tree item", "node title is too long for TreeView")
    })?;
    // TVM_INSERTITEMW copies the null-terminated label into TreeView-owned storage before
    // SendMessageW returns. The mutable pointer type is a Win32 API requirement; this buffer is
    // owned by UiNode, remains alive for this call, and is not mutated here.
    let title_ptr = title.as_ptr().cast_mut();
    let item = TVITEMW {
        mask: TVIF_TEXT | TVIF_PARAM,
        hItem: 0,
        state: 0,
        stateMask: 0,
        pszText: title_ptr,
        cchTextMax: text_len,
        iImage: 0,
        iSelectedImage: 0,
        cChildren: 0,
        lParam: lparam,
    };
    let mut insert = TVINSERTSTRUCTW {
        hParent: parent_handle,
        hInsertAfter: insert_after,
        Anonymous: TVINSERTSTRUCTW_0 { item },
    };

    let handle = SendMessageW(
        tree,
        TVM_INSERTITEMW,
        0,
        &mut insert as *mut TVINSERTSTRUCTW as LPARAM,
    );

    if handle == 0 {
        return Err(last_win32_error("insert tree item"));
    }

    Ok(handle)
}

pub(super) unsafe fn refresh_tree(
    hwnd: HWND,
    state: &mut WindowState,
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    let document = load_ui_document_for_state(state)?;
    apply_tree_document(
        hwnd,
        state,
        document,
        preferred_node_id,
        TreeTabSyncMode::ReloadVisibleContent,
    )
}

pub(super) unsafe fn refresh_tree_after_search_change(
    hwnd: HWND,
    state: &mut WindowState,
    previous_query: &str,
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    let document = match refined_search_document_from_visible_results(state, previous_query) {
        Some(document) => document,
        None => load_ui_document_for_state(state)?,
    };
    apply_tree_document(
        hwnd,
        state,
        document,
        preferred_node_id,
        TreeTabSyncMode::SyncVisibleTabMetadata,
    )
}

pub(super) fn can_refine_search_from_visible_results(
    state: &WindowState,
    previous_query: &str,
    next_query: &str,
) -> bool {
    can_refine_search_document_from_visible_results(
        &state.document,
        state.tree_mode,
        previous_query,
        next_query,
    )
}

fn can_refine_search_document_from_visible_results(
    document: &UiDocument,
    tree_mode: TreeMode,
    previous_query: &str,
    next_query: &str,
) -> bool {
    let previous_query = previous_query.trim();
    let next_query = next_query.trim();
    tree_mode == TreeMode::Active
        && !previous_query.is_empty()
        && !next_query.is_empty()
        && next_query.starts_with(previous_query)
        && search_refinement_preserves_content_scope(previous_query, next_query)
        && document.nodes.len() < SEARCH_RESULT_LIMIT
        && document
            .nodes
            .iter()
            .all(|node| search_result_node_is_refinable(node, previous_query))
}

fn search_refinement_preserves_content_scope(previous_query: &str, next_query: &str) -> bool {
    fts_trigram_content_search_is_enabled(previous_query)
        || !fts_trigram_content_search_is_enabled(next_query)
}

fn fts_trigram_content_search_is_enabled(query: &str) -> bool {
    query.chars().count() >= MIN_FTS_TRIGRAM_CHARS
        && !query.chars().any(|character| character == '\0')
}

unsafe fn apply_tree_document(
    hwnd: HWND,
    state: &mut WindowState,
    document: UiDocument,
    preferred_node_id: Option<i64>,
    tab_sync_mode: TreeTabSyncMode,
) -> Result<(), AppError> {
    let can_reuse_tree_items = can_reuse_trash_tree_items_for_document(state, &document);
    state.document = document;
    match tab_sync_mode {
        TreeTabSyncMode::ReloadVisibleContent => {
            state.sync_tabs_from_visible_document(false)?;
        }
        TreeTabSyncMode::SyncVisibleTabMetadata => {
            state.sync_tabs_from_visible_document_metadata_preserving_content();
        }
        TreeTabSyncMode::PreserveOpenTabContent => {
            state.sync_tabs_from_visible_document_preserving_content();
        }
    }
    if can_reuse_tree_items {
        let selection = selection_for_preferred_or_first_node(state, preferred_node_id)?;
        select_tree_node(hwnd, state, selection)?;
        finish_incremental_tree_refresh(hwnd, state)?;
        return Ok(());
    }
    state.clear_tree_item_handles();
    SendMessageW(state.tree, TVM_DELETEITEM, 0, TVI_ROOT as LPARAM);
    let (selection, item_handles) =
        populate_tree_with_item_handles(state.tree, &state.document, preferred_node_id)?;
    state.replace_tree_item_handles(item_handles);
    select_tree_node(hwnd, state, selection)?;
    refresh_tab_control(state.tab_bar, &state.tabs, &mut state.suppress_tab_change)?;
    update_menu_state(hwnd, state)?;
    update_window_title(hwnd, state)?;
    save_current_tree_refresh_ui_settings(hwnd, state)?;
    Ok(())
}

unsafe fn can_reuse_trash_tree_items_for_document(
    state: &WindowState,
    document: &UiDocument,
) -> bool {
    state.tree_mode == TreeMode::Trash
        && visible_tree_items_match(&state.document, document)
        && tree_item_handles_match_document(state, document)
}

unsafe fn tree_item_handles_match_document(state: &WindowState, document: &UiDocument) -> bool {
    for node in &document.nodes {
        let Some(handle) = state.tree_item_handle_by_node_id(node.id) else {
            return false;
        };
        if tree_item_node_id_if_readable(state, handle) != Some(node.id) {
            return false;
        }
    }
    true
}

fn visible_tree_items_match(current: &UiDocument, next: &UiDocument) -> bool {
    current.nodes.len() == next.nodes.len()
        && current.child_indices_are_display_ordered() == next.child_indices_are_display_ordered()
        && current
            .nodes
            .iter()
            .zip(&next.nodes)
            .all(|(current, next)| visible_tree_node_matches(current, next))
}

fn visible_tree_node_matches(current: &UiNode, next: &UiNode) -> bool {
    current.id == next.id
        && current.parent_id == next.parent_id
        && current.display_parent_id == next.display_parent_id
        && current.title == next.title
        && current.sort_order == next.sort_order
        && current.title_sort_key == next.title_sort_key
        && current.display_title == next.display_title
}

#[derive(Clone, Copy)]
enum TreeTabSyncMode {
    ReloadVisibleContent,
    SyncVisibleTabMetadata,
    PreserveOpenTabContent,
}

unsafe fn refresh_tree_after_active_document_change_preserving_tabs(
    hwnd: HWND,
    state: &mut WindowState,
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    refresh_tree_preserving_open_tab_content(hwnd, state, preferred_node_id)
}

pub(super) unsafe fn refresh_tree_preserving_open_tab_content(
    hwnd: HWND,
    state: &mut WindowState,
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    let document = load_ui_document_for_state(state)?;
    apply_tree_document(
        hwnd,
        state,
        document,
        preferred_node_id,
        TreeTabSyncMode::PreserveOpenTabContent,
    )
}

pub(super) unsafe fn refresh_tree_after_active_document_insert(
    hwnd: HWND,
    state: &mut WindowState,
    node_id: i64,
) -> Result<(), AppError> {
    if can_incrementally_refresh_active_tree(state)
        && refresh_tree_after_active_document_insert_incremental(hwnd, state, node_id)?
    {
        return Ok(());
    }

    refresh_tree_after_active_document_change_preserving_tabs(hwnd, state, Some(node_id))
}

pub(super) unsafe fn refresh_tree_after_active_document_delete(
    hwnd: HWND,
    state: &mut WindowState,
    removed_node_ids: &[i64],
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    if can_incrementally_refresh_active_tree(state)
        && refresh_tree_after_active_document_delete_incremental(
            hwnd,
            state,
            removed_node_ids,
            preferred_node_id,
        )?
    {
        return Ok(());
    }

    refresh_tree_after_active_document_change_preserving_tabs(hwnd, state, preferred_node_id)
}

pub(super) unsafe fn refresh_tree_after_active_document_move(
    hwnd: HWND,
    state: &mut WindowState,
    node_id: i64,
    old_parent_id: Option<i64>,
    new_parent_id: Option<i64>,
) -> Result<(), AppError> {
    if can_incrementally_refresh_active_tree(state)
        && refresh_tree_after_active_document_move_incremental(
            hwnd,
            state,
            node_id,
            old_parent_id,
            new_parent_id,
        )?
    {
        return Ok(());
    }

    refresh_tree_after_active_document_change_preserving_tabs(hwnd, state, Some(node_id))
}

pub(super) unsafe fn refresh_tree_after_label_edit(
    hwnd: HWND,
    state: &mut WindowState,
    preferred_node_id: Option<i64>,
) -> Result<(), AppError> {
    if let Some(node_id) = preferred_node_id {
        if can_incrementally_refresh_active_tree(state)
            && refresh_tree_after_label_edit_incremental(hwnd, state, node_id)?
        {
            return Ok(());
        }
    }

    refresh_tree(hwnd, state, preferred_node_id)
}

fn can_incrementally_refresh_active_tree(state: &WindowState) -> bool {
    state.tree_mode == TreeMode::Active && !search_is_active(state)
}

unsafe fn refresh_tree_after_active_document_insert_incremental(
    hwnd: HWND,
    state: &mut WindowState,
    node_id: i64,
) -> Result<bool, AppError> {
    let parent_id = state
        .app
        .document()
        .node_by_id(node_id)
        .ok_or(DomainError::NodeNotFound { node_id })?
        .parent_id;
    let Some(parent_handle) = tree_parent_handle_for_display_parent(state, parent_id)? else {
        return Ok(false);
    };

    let node_index = state
        .document
        .upsert_active_node_from_document(state.app.document(), node_id)?;
    let Some(insert_after) = tree_insert_after_for_node(state, parent_id, node_id)? else {
        return Ok(false);
    };
    let handle = insert_tree_item(
        state.tree,
        parent_handle,
        insert_after,
        &state.document.nodes[node_index],
    )?;
    state.remember_tree_item_handle(node_id, handle);
    expand_tree_item(state.tree, parent_handle);

    state.sync_tabs_from_visible_document_preserving_content();
    select_tree_node(hwnd, state, Some(InitialSelection { node_index, handle }))?;
    finish_incremental_tree_refresh(hwnd, state)?;
    Ok(true)
}

unsafe fn refresh_tree_after_active_document_delete_incremental(
    hwnd: HWND,
    state: &mut WindowState,
    removed_node_ids: &[i64],
    preferred_node_id: Option<i64>,
) -> Result<bool, AppError> {
    let Some(removed_node_id) = removed_node_ids.first().copied() else {
        return Ok(false);
    };
    let Some(item) = find_tree_item_by_node_id(state, removed_node_id)? else {
        return Ok(false);
    };
    let tree_removed_node_ids = subtree_node_ids(&state.document, removed_node_id);

    if SendMessageW(state.tree, TVM_DELETEITEM, 0, item as LPARAM) == 0 {
        return Err(last_win32_error("delete TreeView item"));
    }
    state.forget_tree_item_handles(&tree_removed_node_ids);
    state.forget_tree_item_handles(removed_node_ids);
    state.document.remove_nodes_by_id(removed_node_ids);
    state.sync_tabs_from_visible_document_preserving_content();

    let selection = selection_for_preferred_or_first_node(state, preferred_node_id)?;
    select_tree_node(hwnd, state, selection)?;
    finish_incremental_tree_refresh(hwnd, state)?;
    Ok(true)
}

unsafe fn refresh_tree_after_active_document_move_incremental(
    hwnd: HWND,
    state: &mut WindowState,
    node_id: i64,
    old_parent_id: Option<i64>,
    requested_new_parent_id: Option<i64>,
) -> Result<bool, AppError> {
    let Some(item) = find_tree_item_by_node_id(state, node_id)? else {
        return Ok(false);
    };
    let moved_tree_node_ids = moved_subtree_node_ids(&state.document, node_id);
    let new_parent_id = state
        .app
        .document()
        .node_by_id(node_id)
        .ok_or(DomainError::NodeNotFound { node_id })?
        .parent_id;
    let Some(parent_handle) = tree_parent_handle_for_display_parent(state, new_parent_id)? else {
        return Ok(false);
    };

    state.document.sync_active_nodes_after_move(
        state.app.document(),
        node_id,
        old_parent_id,
        requested_new_parent_id.or(new_parent_id),
    )?;
    let Some(insert_after) = tree_insert_after_for_node(state, new_parent_id, node_id)? else {
        return Ok(false);
    };

    if SendMessageW(state.tree, TVM_DELETEITEM, 0, item as LPARAM) == 0 {
        return Err(last_win32_error("delete moved TreeView item"));
    }
    state.forget_tree_item_handles(&moved_tree_node_ids);
    let (selection, item_handles) = insert_tree_subtree(
        state.tree,
        &state.document,
        node_id,
        parent_handle,
        insert_after,
        Some(node_id),
    )?;
    state.extend_tree_item_handles(item_handles);
    expand_tree_item(state.tree, parent_handle);

    state.sync_tabs_from_visible_document_preserving_content();
    select_tree_node(hwnd, state, selection)?;
    finish_incremental_tree_refresh(hwnd, state)?;
    Ok(true)
}

unsafe fn refresh_tree_after_label_edit_incremental(
    hwnd: HWND,
    state: &mut WindowState,
    node_id: i64,
) -> Result<bool, AppError> {
    let Some(item) = verified_cached_tree_item_for_label_edit(state, node_id) else {
        return Ok(false);
    };

    let node_index = state
        .document
        .upsert_active_node_from_document(state.app.document(), node_id)?;
    set_tree_item_text(state.tree, item, &state.document.nodes[node_index])?;

    state.sync_tabs_from_visible_document_preserving_content();
    finish_incremental_tree_refresh(hwnd, state)?;
    Ok(true)
}

unsafe fn verified_cached_tree_item_for_label_edit(
    state: &WindowState,
    node_id: i64,
) -> Option<HTREEITEM> {
    let item = state.tree_item_handle_by_node_id(node_id)?;
    (tree_item_node_id_if_readable(state, item) == Some(node_id)).then_some(item)
}

unsafe fn tree_item_node_id_if_readable(
    state: &WindowState,
    item_handle: HTREEITEM,
) -> Option<i64> {
    let mut item = TVITEMW {
        mask: TVIF_PARAM,
        hItem: item_handle,
        state: 0,
        stateMask: 0,
        pszText: ptr::null_mut(),
        cchTextMax: 0,
        iImage: 0,
        iSelectedImage: 0,
        cChildren: 0,
        lParam: 0,
    };

    let found = SendMessageW(
        state.tree,
        TVM_GETITEMW,
        0,
        &mut item as *mut TVITEMW as LPARAM,
    );
    if found == 0 {
        return None;
    }

    node_id_from_lparam(item.lParam)
}

unsafe fn set_tree_item_text(
    tree: HWND,
    item_handle: HTREEITEM,
    node: &UiNode,
) -> Result<(), AppError> {
    let title = node.display_title.as_slice();
    let text_len = i32::try_from(title.len()).map_err(|_| {
        AppError::platform(
            "set TreeView item text",
            "node title is too long for TreeView",
        )
    })?;
    // TVM_SETITEMW copies the null-terminated label into TreeView-owned storage before
    // SendMessageW returns. The mutable pointer type is a Win32 API requirement; this buffer is
    // owned by UiNode, remains alive for this call, and is not mutated here.
    let title_ptr = title.as_ptr().cast_mut();
    let mut item = TVITEMW {
        mask: TVIF_TEXT,
        hItem: item_handle,
        state: 0,
        stateMask: 0,
        pszText: title_ptr,
        cchTextMax: text_len,
        iImage: 0,
        iSelectedImage: 0,
        cChildren: 0,
        lParam: 0,
    };

    let updated = SendMessageW(tree, TVM_SETITEMW, 0, &mut item as *mut TVITEMW as LPARAM);
    if updated == 0 {
        return Err(last_win32_error("set TreeView item text"));
    }

    Ok(())
}

unsafe fn tree_parent_handle_for_display_parent(
    state: &WindowState,
    parent_id: Option<i64>,
) -> Result<Option<HTREEITEM>, AppError> {
    match parent_id {
        Some(parent_id) => find_tree_item_by_node_id(state, parent_id),
        None => Ok(Some(TVI_ROOT)),
    }
}

unsafe fn tree_insert_after_for_node(
    state: &WindowState,
    parent_id: Option<i64>,
    node_id: i64,
) -> Result<Option<HTREEITEM>, AppError> {
    if let Some(child_range) = state.document.display_ordered_child_index_range(parent_id) {
        let Some(previous_node_id) =
            previous_sibling_node_id_in_range(&state.document, child_range, node_id)
        else {
            return Ok(Some(TVI_FIRST));
        };
        return find_tree_item_by_node_id(state, previous_node_id);
    }

    let child_indices = ordered_child_indices(&state.document, parent_id);
    tree_insert_after_for_ordered_child_indices(state, &child_indices, node_id)
}

unsafe fn tree_insert_after_for_ordered_child_indices(
    state: &WindowState,
    child_indices: &[usize],
    node_id: i64,
) -> Result<Option<HTREEITEM>, AppError> {
    let Some(previous_node_id) = previous_sibling_node_id(&state.document, child_indices, node_id)
    else {
        return Ok(Some(TVI_FIRST));
    };
    find_tree_item_by_node_id(state, previous_node_id)
}

fn previous_sibling_node_id(
    document: &UiDocument,
    child_indices: &[usize],
    node_id: i64,
) -> Option<i64> {
    let mut previous_node_id = None;
    for &node_index in child_indices {
        let sibling_id = document.nodes[node_index].id;
        if sibling_id == node_id {
            break;
        }
        previous_node_id = Some(sibling_id);
    }
    previous_node_id
}

fn previous_sibling_node_id_in_range(
    document: &UiDocument,
    child_range: std::ops::Range<usize>,
    node_id: i64,
) -> Option<i64> {
    let mut previous_node_id = None;
    for node_index in child_range {
        let sibling_id = document.nodes[node_index].id;
        if sibling_id == node_id {
            break;
        }
        previous_node_id = Some(sibling_id);
    }
    previous_node_id
}

unsafe fn insert_tree_subtree(
    tree: HWND,
    document: &UiDocument,
    node_id: i64,
    parent_handle: HTREEITEM,
    insert_after: HTREEITEM,
    preferred_node_id: Option<i64>,
) -> Result<(Option<InitialSelection>, TreeItemHandlesByNodeId), AppError> {
    let Some(node_index) = document.node_index_by_id(node_id) else {
        return Ok((None, TreeItemHandlesByNodeId::new()));
    };
    let mut population = TreePopulation::new(preferred_node_id, 1);
    let node = &document.nodes[node_index];
    let handle = insert_tree_item(tree, parent_handle, insert_after, node)?;
    population.remember(node, node_index, handle);
    if document.child_indices_are_display_ordered() {
        insert_child_items_from_display_ordered_ranges(
            tree,
            document,
            Some(node_id),
            handle,
            &mut population,
            TreeExpansion::All,
        )?;
    } else {
        let child_indices_by_parent = display_child_indices_by_parent(document);
        insert_child_items(
            tree,
            document,
            &child_indices_by_parent,
            Some(node_id),
            handle,
            &mut population,
            TreeExpansion::All,
        )?;
    }
    expand_tree_item(tree, handle);
    Ok(population.into_parts())
}

unsafe fn insert_child_items_from_display_ordered_ranges(
    tree: HWND,
    document: &UiDocument,
    parent_id: Option<i64>,
    parent_handle: HTREEITEM,
    population: &mut TreePopulation,
    expansion: TreeExpansion<'_>,
) -> Result<(), AppError> {
    enum InsertTask {
        InsertChildren {
            parent_id: Option<i64>,
            parent_handle: HTREEITEM,
        },
        InsertNode {
            parent_handle: HTREEITEM,
            node_index: usize,
        },
        ExpandFolder {
            handle: HTREEITEM,
        },
    }

    let mut tasks = vec![InsertTask::InsertChildren {
        parent_id,
        parent_handle,
    }];

    while let Some(task) = tasks.pop() {
        match task {
            InsertTask::InsertChildren {
                parent_id,
                parent_handle,
            } => {
                let Some(children) = document.display_ordered_child_index_range(parent_id) else {
                    continue;
                };
                for node_index in children.rev() {
                    tasks.push(InsertTask::InsertNode {
                        parent_handle,
                        node_index,
                    });
                }
            }
            InsertTask::InsertNode {
                parent_handle,
                node_index,
            } => {
                let node = &document.nodes[node_index];
                let handle = insert_tree_item(tree, parent_handle, TVI_LAST, node)?;

                population.remember(node, node_index, handle);

                if expansion.should_expand(node.id) {
                    tasks.push(InsertTask::ExpandFolder { handle });
                }

                tasks.push(InsertTask::InsertChildren {
                    parent_id: Some(node.id),
                    parent_handle: handle,
                });
            }
            InsertTask::ExpandFolder { handle } => {
                SendMessageW(tree, TVM_EXPAND, TVE_EXPAND as WPARAM, handle as LPARAM);
            }
        }
    }

    Ok(())
}

unsafe fn expand_tree_item(tree: HWND, item: HTREEITEM) {
    if item != TVI_ROOT {
        SendMessageW(tree, TVM_EXPAND, TVE_EXPAND as WPARAM, item as LPARAM);
    }
}

unsafe fn selection_for_preferred_or_first_node(
    state: &WindowState,
    preferred_node_id: Option<i64>,
) -> Result<Option<InitialSelection>, AppError> {
    if let Some(preferred_node_id) = preferred_node_id {
        if let Some(selection) = selection_for_visible_node_id(state, preferred_node_id)? {
            return Ok(Some(selection));
        }
    }

    let root = SendMessageW(state.tree, TVM_GETNEXTITEM, TVGN_ROOT as WPARAM, 0);
    selection_for_tree_item(state, root)
}

unsafe fn selection_for_visible_node_id(
    state: &WindowState,
    node_id: i64,
) -> Result<Option<InitialSelection>, AppError> {
    let Some(handle) = find_tree_item_by_node_id(state, node_id)? else {
        return Ok(None);
    };
    selection_for_tree_item(state, handle)
}

unsafe fn selection_for_tree_item(
    state: &WindowState,
    handle: HTREEITEM,
) -> Result<Option<InitialSelection>, AppError> {
    if handle == 0 {
        return Ok(None);
    }
    let Some(node_id) = tree_node_id_from_item(state, handle)? else {
        return Ok(None);
    };
    let Some(node_index) = state.document.node_index_by_id(node_id) else {
        return Ok(None);
    };
    Ok(Some(InitialSelection { node_index, handle }))
}

unsafe fn finish_incremental_tree_refresh(
    hwnd: HWND,
    state: &mut WindowState,
) -> Result<(), AppError> {
    refresh_tab_control(state.tab_bar, &state.tabs, &mut state.suppress_tab_change)?;
    update_menu_state(hwnd, state)?;
    update_window_title(hwnd, state)?;
    save_current_tree_refresh_ui_settings(hwnd, state)?;
    Ok(())
}

fn load_ui_document_for_state(state: &WindowState) -> Result<UiDocument, AppError> {
    let language = state.app.ui_settings().language;
    match state.tree_mode {
        TreeMode::Active if search_is_active(state) => {
            let results = state.app.search_documents(&state.search_query)?;
            UiDocument::from_search_results(results, language)
        }
        TreeMode::Active => UiDocument::from_active_document(state.app.document()),
        TreeMode::Trash => UiDocument::from_trash_nodes(&state.app.deleted_nodes()?, language),
    }
}

fn refined_search_document_from_visible_results(
    state: &WindowState,
    previous_query: &str,
) -> Option<UiDocument> {
    let next_query = state.search_query.trim();
    if !can_refine_search_from_visible_results(state, previous_query, next_query) {
        return None;
    }

    let nodes = state
        .document
        .nodes
        .iter()
        .filter(|node| search_result_node_matches(node, next_query))
        .map(clone_ui_node)
        .collect();

    Some(UiDocument::from_ui_nodes(nodes))
}

fn search_result_node_matches(node: &UiNode, query: &str) -> bool {
    contains_sqlite_like_literal(&node.title, query)
}

fn search_result_node_is_refinable(node: &UiNode, previous_query: &str) -> bool {
    matches!(node.source, DocumentTabSource::SearchResult)
        && !node.search_content_matched
        && contains_sqlite_like_literal(&node.title, previous_query)
}

fn contains_sqlite_like_literal(text: &str, query: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }

    let query = query.as_bytes();
    text.as_bytes()
        .windows(query.len())
        .any(|window| window.eq_ignore_ascii_case(query))
}

fn clone_ui_node(node: &UiNode) -> UiNode {
    UiNode {
        id: node.id,
        parent_id: node.parent_id,
        display_parent_id: node.display_parent_id,
        title: node.title.clone(),
        sort_order: node.sort_order,
        title_sort_key: node.title_sort_key.clone(),
        display_title: node.display_title.clone(),
        search_content_matched: node.search_content_matched,
        updated_at: node.updated_at.clone(),
        editable: node.editable,
        source: node.source,
    }
}

fn display_child_indices_by_parent(document: &UiDocument) -> ChildIndicesByParent {
    let mut child_indices_by_parent = ChildIndicesByParent::with_capacity(document.nodes.len());
    for (index, node) in document.nodes.iter().enumerate() {
        child_indices_by_parent
            .entry(node.display_parent_id)
            .or_default()
            .push(index);
    }
    if !document.child_indices_are_display_ordered() {
        for child_indices in child_indices_by_parent.values_mut() {
            child_indices.sort_by(|left, right| {
                compare_ui_nodes_for_display(&document.nodes[*left], &document.nodes[*right])
            });
        }
    }
    child_indices_by_parent
}

fn ordered_child_indices(document: &UiDocument, parent_id: Option<i64>) -> Vec<usize> {
    if let Some(child_range) = document.display_ordered_child_index_range(parent_id) {
        return child_range.collect();
    }

    let mut child_indices: Vec<usize> = document
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.display_parent_id == parent_id)
        .map(|(index, _)| index)
        .collect();
    if !document.child_indices_are_display_ordered() {
        child_indices.sort_by(|left, right| {
            compare_ui_nodes_for_display(&document.nodes[*left], &document.nodes[*right])
        });
    }
    child_indices
}

pub(super) fn search_is_active(state: &WindowState) -> bool {
    state.tree_mode == TreeMode::Active && !state.search_query.trim().is_empty()
}

pub(super) unsafe fn select_tree_node(
    hwnd: HWND,
    state: &mut WindowState,
    selection: Option<InitialSelection>,
) -> Result<(), AppError> {
    if let Some(selection) = selection {
        let target_node_id = tree_selection_node_id(state, selection.node_index)?;
        if programmatic_tree_selection_changes_active_tab(
            state.tabs.active().map(|tab| tab.node_id),
            target_node_id,
        ) && !autosave_active_tab_before_navigation(hwnd, state)?
        {
            return Ok(());
        }
        let Some(selection) = selection_for_visible_node_id(state, target_node_id)? else {
            return Ok(());
        };

        state.selected_node_id = Some(target_node_id);
        state.open_or_activate_tab_from_node(selection.node_index)?;
        state.suppress_selection_change = true;
        SendMessageW(
            state.tree,
            TVM_SELECTITEM,
            TVGN_CARET as WPARAM,
            selection.handle as LPARAM,
        );
        state.suppress_selection_change = false;
    } else {
        state.selected_node_id = None;
        if !state.tabs.has_active() {
            state.show_active_tab_in_editor()?;
        }
    }
    refresh_tab_control(state.tab_bar, &state.tabs, &mut state.suppress_tab_change)?;
    update_window_title(hwnd, state)?;
    Ok(())
}

fn tree_selection_node_id(state: &WindowState, node_index: usize) -> Result<i64, AppError> {
    state
        .document
        .nodes
        .get(node_index)
        .map(|node| node.id)
        .ok_or_else(|| {
            AppError::platform(
                "select tree node",
                "tree selection node index was not found",
            )
        })
}

fn programmatic_tree_selection_changes_active_tab(
    active_tab_node_id: Option<i64>,
    target_node_id: i64,
) -> bool {
    match active_tab_node_id {
        Some(active_tab_node_id) => active_tab_node_id != target_node_id,
        None => false,
    }
}

pub(super) unsafe fn start_selected_label_edit(state: &WindowState) -> Result<(), AppError> {
    let selected_handle = SendMessageW(state.tree, TVM_GETNEXTITEM, TVGN_CARET as WPARAM, 0);
    if selected_handle == 0 {
        return Err(DomainError::NodeNotFound { node_id: 0 }.into());
    }

    let edit = SendMessageW(state.tree, TVM_EDITLABELW, 0, selected_handle as LPARAM);
    if edit == 0 {
        return Err(last_win32_error("start TreeView label edit"));
    }

    Ok(())
}

unsafe fn handle_tree_begin_drag(hwnd: HWND, state: &mut WindowState, lparam: LPARAM) -> LRESULT {
    if state.tree_mode != TreeMode::Active || search_is_active(state) {
        return 0;
    }

    let Some(tree_view) = notify_ptr_from_lparam::<NMTREEVIEWW>(lparam) else {
        return 0;
    };

    let Some(node_id) = node_id_from_lparam((*tree_view).itemNew.lParam) else {
        return 0;
    };

    let Some(node) = state.document.node_by_id(node_id) else {
        return 0;
    };

    state.dragging_node_id = Some(node.id);
    set_tree_drop_highlight(state, Some((*tree_view).itemNew.hItem));
    SetCapture(hwnd);
    0
}

pub(super) unsafe fn handle_tree_drag_over(state: &mut WindowState, lparam: LPARAM) {
    if state.dragging_node_id.is_none() {
        return;
    }

    let target = tree_item_at_lparam(state, lparam);
    set_tree_drop_highlight(state, target);
}

pub(super) unsafe fn handle_tree_drop(
    hwnd: HWND,
    state: &mut WindowState,
    lparam: LPARAM,
) -> Result<(), AppError> {
    if state.tree_mode != TreeMode::Active || search_is_active(state) {
        clear_tree_drag(state);
        return Ok(());
    }

    let Some(source_node_id) = state.dragging_node_id else {
        return Ok(());
    };

    let target_node_id = match tree_item_at_lparam(state, lparam) {
        Some(item) => tree_node_id_from_item(state, item)?,
        None => None,
    };
    clear_tree_drag(state);

    let Some(target_node_id) = target_node_id else {
        return Ok(());
    };
    if source_node_id == target_node_id {
        return Ok(());
    }

    if !resolve_dirty_before_refresh(hwnd, state)? {
        update_menu_state(hwnd, state)?;
        return Ok(());
    }

    let Some(source_parent_id) = state
        .document
        .node_by_id(source_node_id)
        .map(|node| node.parent_id)
    else {
        return Err(DomainError::NodeNotFound {
            node_id: source_node_id,
        }
        .into());
    };
    if state.document.node_by_id(target_node_id).is_none() {
        return Err(DomainError::NodeNotFound {
            node_id: target_node_id,
        }
        .into());
    };
    state
        .app
        .move_node_to_parent_end(source_node_id, target_node_id)?;

    state.sync_tabs_from_active_document_local_metadata(true)?;
    refresh_tab_control(state.tab_bar, &state.tabs, &mut state.suppress_tab_change)?;
    refresh_tree_after_active_document_move(
        hwnd,
        state,
        source_node_id,
        source_parent_id,
        Some(target_node_id),
    )
}

pub(super) unsafe fn clear_tree_drag(state: &mut WindowState) {
    set_tree_drop_highlight(state, None);
    state.dragging_node_id = None;
    ReleaseCapture();
}

unsafe fn set_tree_drop_highlight(state: &mut WindowState, item: Option<HTREEITEM>) {
    if state.drag_highlight == item {
        return;
    }

    let item = item.unwrap_or(0);
    SendMessageW(
        state.tree,
        TVM_SELECTITEM,
        TVGN_DROPHILITE as WPARAM,
        item as LPARAM,
    );
    state.drag_highlight = (item != 0).then_some(item);
}

unsafe fn tree_item_at_lparam(state: &WindowState, lparam: LPARAM) -> Option<HTREEITEM> {
    let mut point = client_point_from_lparam(lparam);
    let tree_y = tree_top_offset(state);
    if point.x < 0 || point.y < tree_y || point.x >= state.split_width {
        return None;
    }
    point.y -= tree_y;

    tree_item_at_tree_point(state, point)
}

unsafe fn tree_item_at_tree_point(state: &WindowState, point: POINT) -> Option<HTREEITEM> {
    let mut hit_test = TVHITTESTINFO {
        pt: point,
        flags: 0,
        hItem: 0,
    };
    let item = SendMessageW(
        state.tree,
        TVM_HITTEST,
        0,
        &mut hit_test as *mut TVHITTESTINFO as LPARAM,
    );

    (item != 0).then_some(item)
}

unsafe fn tree_node_id_from_item(
    state: &WindowState,
    item_handle: HTREEITEM,
) -> Result<Option<i64>, AppError> {
    let mut item = TVITEMW {
        mask: TVIF_PARAM,
        hItem: item_handle,
        state: 0,
        stateMask: 0,
        pszText: ptr::null_mut(),
        cchTextMax: 0,
        iImage: 0,
        iSelectedImage: 0,
        cChildren: 0,
        lParam: 0,
    };

    let found = SendMessageW(
        state.tree,
        TVM_GETITEMW,
        0,
        &mut item as *mut TVITEMW as LPARAM,
    );
    if found == 0 {
        return Err(last_win32_error("read TreeView item"));
    }

    let Some(node_id) = node_id_from_lparam(item.lParam) else {
        return Ok(None);
    };
    Ok(state.document.contains_node_id(node_id).then_some(node_id))
}

fn node_id_from_lparam(lparam: LPARAM) -> Option<i64> {
    let node_id = i64::try_from(lparam).ok()?;
    (node_id >= 0).then_some(node_id)
}

pub(super) unsafe fn select_visible_tree_node_by_id(
    state: &mut WindowState,
    node_id: i64,
) -> Result<(), AppError> {
    if !state.document.contains_node_id(node_id) {
        return Ok(());
    }

    let Some(item) = find_tree_item_by_node_id(state, node_id)? else {
        return Ok(());
    };

    state.selected_node_id = Some(node_id);
    state.suppress_selection_change = true;
    SendMessageW(
        state.tree,
        TVM_SELECTITEM,
        TVGN_CARET as WPARAM,
        item as LPARAM,
    );
    state.suppress_selection_change = false;
    Ok(())
}

pub(super) unsafe fn select_tree_item_at_screen_point(
    hwnd: HWND,
    state: &mut WindowState,
    mut point: POINT,
) -> Result<bool, AppError> {
    if ScreenToClient(state.tree, &mut point) == 0 {
        return Err(last_win32_error("convert TreeView context menu point"));
    }

    let Some(item) = tree_item_at_tree_point(state, point) else {
        return Ok(true);
    };
    let Some(node_id) = tree_node_id_from_item(state, item)? else {
        return Ok(true);
    };
    if state.selected_node_id == Some(node_id) {
        return Ok(true);
    }

    let Some(node_index) = state.document.node_index_by_id(node_id) else {
        return Err(DomainError::NodeNotFound { node_id }.into());
    };

    if !autosave_active_tab_before_navigation(hwnd, state)? {
        return Ok(false);
    }

    select_tree_node(
        hwnd,
        state,
        Some(InitialSelection {
            node_index,
            handle: item,
        }),
    )?;
    update_menu_state(hwnd, state)?;
    save_current_ui_settings(hwnd, state)?;
    Ok(true)
}

unsafe fn find_tree_item_by_node_id(
    state: &WindowState,
    node_id: i64,
) -> Result<Option<HTREEITEM>, AppError> {
    if let Some(handle) = state.tree_item_handle_by_node_id(node_id) {
        return Ok(Some(handle));
    }

    let root = SendMessageW(state.tree, TVM_GETNEXTITEM, TVGN_ROOT as WPARAM, 0);
    find_tree_item_in_siblings(state, root, node_id)
}

unsafe fn find_tree_item_in_siblings(
    state: &WindowState,
    item: HTREEITEM,
    node_id: i64,
) -> Result<Option<HTREEITEM>, AppError> {
    let mut items = Vec::new();
    if item != 0 {
        items.push(item);
    }

    while let Some(item) = items.pop() {
        if tree_node_id_from_item(state, item)? == Some(node_id) {
            return Ok(Some(item));
        }

        let next = SendMessageW(
            state.tree,
            TVM_GETNEXTITEM,
            TVGN_NEXT as WPARAM,
            item as LPARAM,
        );
        if next != 0 {
            items.push(next);
        }

        let child = SendMessageW(
            state.tree,
            TVM_GETNEXTITEM,
            TVGN_CHILD as WPARAM,
            item as LPARAM,
        );
        if child != 0 {
            items.push(child);
        }
    }

    Ok(None)
}

pub(super) fn subtree_node_ids(document: &UiDocument, root_node_id: i64) -> Vec<i64> {
    let child_ids_by_parent_map = child_ids_by_parent(document);
    let mut ids = Vec::new();
    collect_subtree_node_ids(root_node_id, &child_ids_by_parent_map, &mut ids);
    ids
}

fn moved_subtree_node_ids(document: &UiDocument, root_node_id: i64) -> Vec<i64> {
    if document.child_indices_are_display_ordered() {
        return display_ordered_subtree_node_ids(document, root_node_id);
    }

    subtree_node_ids(document, root_node_id)
}

fn display_ordered_subtree_node_ids(document: &UiDocument, root_node_id: i64) -> Vec<i64> {
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    let mut pending = vec![root_node_id];

    while let Some(node_id) = pending.pop() {
        if !seen.insert(node_id) {
            continue;
        }

        ids.push(node_id);
        if let Some(children) = document.display_ordered_child_index_range(Some(node_id)) {
            pending.extend(children.rev().map(|index| document.nodes[index].id));
        }
    }

    ids
}

fn collect_subtree_node_ids(
    node_id: i64,
    child_ids_by_parent: &ChildIdsByParent,
    ids: &mut Vec<i64>,
) {
    let mut seen = HashSet::new();
    let mut pending = vec![node_id];

    while let Some(node_id) = pending.pop() {
        if !seen.insert(node_id) {
            continue;
        }

        ids.push(node_id);
        if let Some(children) = child_ids_by_parent.get(&node_id) {
            pending.extend(children.iter().rev().copied());
        }
    }
}

fn child_ids_by_parent(document: &UiDocument) -> ChildIdsByParent {
    let mut child_ids_by_parent = ChildIdsByParent::with_capacity(document.nodes.len());
    for node in &document.nodes {
        if let Some(parent_id) = node.parent_id {
            child_ids_by_parent
                .entry(parent_id)
                .or_default()
                .push(node.id);
        }
    }
    child_ids_by_parent
}

pub(super) fn notify_header_from_lparam(lparam: LPARAM) -> Option<*const NMHDR> {
    notify_ptr_from_lparam::<NMHDR>(lparam)
}

fn notify_ptr_from_lparam<T>(lparam: LPARAM) -> Option<*const T> {
    if lparam == 0 {
        return None;
    }

    let pointer = lparam as *const T;
    if pointer.is_null() || !(pointer as usize).is_multiple_of(mem::align_of::<T>()) {
        return None;
    }

    pointer_is_readable(pointer.cast::<c_void>(), mem::size_of::<T>()).then_some(pointer)
}

fn pointer_is_readable(pointer: *const c_void, size: usize) -> bool {
    let mut memory = MemoryBasicInformation {
        base_address: ptr::null_mut(),
        allocation_base: ptr::null_mut(),
        allocation_protect: 0,
        partition_id: 0,
        region_size: 0,
        state: 0,
        protect: 0,
        type_: 0,
    };

    // SAFETY: VirtualQuery reads process memory metadata for the supplied address and writes the
    // result into the initialized local MEMORY_BASIC_INFORMATION-compatible buffer.
    let queried = unsafe {
        VirtualQuery(
            pointer,
            &mut memory,
            mem::size_of::<MemoryBasicInformation>(),
        )
    };
    if queried == 0
        || memory.state != MEM_COMMIT
        || memory.protect & (PAGE_NOACCESS | PAGE_GUARD) != 0
        || memory.protect & PAGE_READABLE_MASK == 0
    {
        return false;
    }

    let start = pointer as usize;
    let Some(end) = start.checked_add(size) else {
        return false;
    };
    let region_start = memory.base_address as usize;
    let Some(region_end) = region_start.checked_add(memory.region_size) else {
        return false;
    };

    region_start <= start && end <= region_end
}

pub(super) unsafe fn handle_notify(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if wparam != CONTROL_TREE_ID {
        return 0;
    }

    let Some(header) = notify_header_from_lparam(lparam) else {
        return 0;
    };

    let Some(mut state) = window_state(hwnd) else {
        return 0;
    };
    // SAFETY: notify_header_from_lparam checked that lparam covers an NMHDR before fields are read.
    if (*header).hwndFrom != state.tree {
        return 0;
    }

    match (*header).code {
        TVN_SELCHANGINGW => handle_tree_selection_changing(hwnd, &mut state, lparam),
        TVN_SELCHANGEDW => {
            handle_tree_selection_changed(hwnd, &mut state, lparam);
            0
        }
        TVN_BEGINLABELEDITW => handle_tree_begin_label_edit(hwnd, &mut state, lparam),
        TVN_ENDLABELEDITW => handle_tree_end_label_edit(hwnd, &mut state, lparam),
        TVN_BEGINDRAGW => handle_tree_begin_drag(hwnd, &mut state, lparam),
        _ => 0,
    }
}

unsafe fn handle_tree_selection_changing(
    hwnd: HWND,
    state: &mut WindowState,
    lparam: LPARAM,
) -> LRESULT {
    if state.suppress_selection_change {
        return 0;
    }

    let Some(tree_view) = notify_ptr_from_lparam::<NMTREEVIEWW>(lparam) else {
        return 0;
    };

    let Some(new_node_id) = node_id_from_lparam((*tree_view).itemNew.lParam) else {
        return 0;
    };
    if Some(new_node_id) == state.selected_node_id {
        return 0;
    }

    match autosave_active_tab_before_navigation(hwnd, state) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(error) => {
            show_app_error_for_language(state.app.ui_settings().language, &error);
            1
        }
    }
}

unsafe fn handle_tree_selection_changed(hwnd: HWND, state: &mut WindowState, lparam: LPARAM) {
    if state.suppress_selection_change {
        return;
    }

    let Some(tree_view) = notify_ptr_from_lparam::<NMTREEVIEWW>(lparam) else {
        return;
    };

    let Some(node_id) = node_id_from_lparam((*tree_view).itemNew.lParam) else {
        state.selected_node_id = None;
        if !state.tabs.has_active() {
            if let Err(error) = state.show_active_tab_in_editor() {
                show_app_error_for_language(state.app.ui_settings().language, &error);
            }
        }
        if let Err(error) = update_menu_state(hwnd, state) {
            show_app_error_for_language(state.app.ui_settings().language, &error);
        }
        if let Err(error) = update_window_title(hwnd, state) {
            show_app_error_for_language(state.app.ui_settings().language, &error);
        }
        return;
    };

    let Some(node_index) = state.document.node_index_by_id(node_id) else {
        let error: AppError = DomainError::NodeNotFound { node_id }.into();
        show_app_error_for_language(state.app.ui_settings().language, &error);
        return;
    };
    state.selected_node_id = Some(node_id);
    if let Err(error) = state.open_or_activate_tab_from_node(node_index) {
        show_app_error_for_language(state.app.ui_settings().language, &error);
    }
    if let Err(error) =
        refresh_tab_control(state.tab_bar, &state.tabs, &mut state.suppress_tab_change)
    {
        show_app_error_for_language(state.app.ui_settings().language, &error);
    }
    if let Err(error) = update_menu_state(hwnd, state) {
        show_app_error_for_language(state.app.ui_settings().language, &error);
    }
    if let Err(error) = update_window_title(hwnd, state) {
        show_app_error_for_language(state.app.ui_settings().language, &error);
    }
}

unsafe fn handle_tree_begin_label_edit(
    hwnd: HWND,
    state: &mut WindowState,
    lparam: LPARAM,
) -> LRESULT {
    if state.tree_mode != TreeMode::Active || search_is_active(state) {
        return 1;
    }

    let Some(display_info) = notify_ptr_from_lparam::<NMTVDISPINFOW>(lparam) else {
        return 1;
    };

    let Some(node_id) = node_id_from_lparam((*display_info).item.lParam) else {
        return 1;
    };

    let Some(node) = state.document.node_by_id(node_id) else {
        return 1;
    };
    state.label_edit = Some(TreeLabelEditState::new(node.id));

    let edit = SendMessageW(state.tree, TVM_GETEDITCONTROL, 0, 0) as HWND;
    if edit.is_null() {
        state.label_edit = None;
        show_app_error_for_language(
            state.app.ui_settings().language,
            &AppError::platform(
                "start TreeView label edit",
                "TreeView edit control was not created",
            ),
        );
        return 1;
    }

    let title = utf8_to_wide_null_lossy(&node.title);
    if SetWindowTextW(edit, title.as_ptr()) == 0 {
        state.label_edit = None;
        let error = last_win32_error("set TreeView label edit text");
        show_app_error_for_language(state.app.ui_settings().language, &error);
        return 1;
    }

    if let Err(error) = subclass_tree_label_edit(edit, hwnd) {
        state.label_edit = None;
        show_app_error_for_language(state.app.ui_settings().language, &error);
        return 1;
    }

    0
}

unsafe fn handle_tree_end_label_edit(
    hwnd: HWND,
    state: &mut WindowState,
    lparam: LPARAM,
) -> LRESULT {
    let Some(display_info) = notify_ptr_from_lparam::<NMTVDISPINFOW>(lparam) else {
        state.label_edit = None;
        return 0;
    };

    let text = (*display_info).item.pszText;
    let notification_title = if text.is_null() {
        None
    } else {
        Some(wide_null_to_string(text))
    };
    let fallback_node_id = {
        node_id_from_lparam((*display_info).item.lParam)
            .and_then(|node_id| state.document.node_by_id(node_id).map(|node| node.id))
    };
    let Some((node_id, title)) = resolve_label_edit_commit(
        state.label_edit.take(),
        notification_title,
        fallback_node_id,
    ) else {
        return 0;
    };

    match resolve_dirty_before_refresh(hwnd, state) {
        Ok(true) => {}
        Ok(false) => {
            if let Err(error) = update_menu_state(hwnd, state) {
                show_app_error_for_language(state.app.ui_settings().language, &error);
            }
            return 0;
        }
        Err(error) => {
            show_app_error_for_language(state.app.ui_settings().language, &error);
            return 0;
        }
    }

    match state.app.rename_node(node_id, &title) {
        Ok(()) => {
            if let Err(error) = state.sync_tabs_from_active_document_local_metadata(true) {
                show_app_error_for_language(state.app.ui_settings().language, &error);
                return 0;
            }
            if let Err(error) = queue_tree_refresh_after_label_edit(hwnd, state, node_id) {
                show_app_error_for_language(state.app.ui_settings().language, &error);
            }
            end_label_edit_reject_control_text()
        }
        Err(error) => {
            show_app_error_for_language(state.app.ui_settings().language, &error);
            0
        }
    }
}

unsafe fn queue_tree_refresh_after_label_edit(
    hwnd: HWND,
    state: &mut WindowState,
    node_id: i64,
) -> Result<(), AppError> {
    state.pending_tree_label_edit_refresh_node_id = Some(node_id);
    let posted = PostMessageW(hwnd, WM_APP_REFRESH_TREE_AFTER_LABEL_EDIT, 0, 0);
    if posted == 0 {
        state.pending_tree_label_edit_refresh_node_id = None;
        return Err(last_win32_error("queue tree refresh after label edit"));
    }

    Ok(())
}

fn end_label_edit_reject_control_text() -> LRESULT {
    // For TVN_ENDLABELEDITW, FALSE prevents the common control from copying pszText after this
    // handler returns. The app has already committed the rename and queued an app-controlled
    // metadata/text refresh.
    END_LABEL_EDIT_REJECT_CONTROL_TEXT
}

unsafe fn subclass_tree_label_edit(edit: HWND, parent: HWND) -> Result<(), AppError> {
    if SetWindowSubclass(
        edit,
        Some(tree_label_edit_subclass_proc),
        TREE_LABEL_EDIT_SUBCLASS_ID,
        parent as usize,
    ) == 0
    {
        return Err(last_win32_error("subclass TreeView label edit"));
    }

    Ok(())
}

unsafe extern "system" fn tree_label_edit_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    subclass_data: usize,
) -> LRESULT {
    let parent = subclass_data as HWND;

    match message {
        WM_KEYDOWN if wparam == VK_ESCAPE_KEY => remember_tree_label_edit_cancel(parent),
        WM_KILLFOCUS => remember_tree_label_edit_focus_loss_text(hwnd, parent),
        WM_NCDESTROY => {
            let result = DefSubclassProc(hwnd, message, wparam, lparam);
            RemoveWindowSubclass(
                hwnd,
                Some(tree_label_edit_subclass_proc),
                TREE_LABEL_EDIT_SUBCLASS_ID,
            );
            return result;
        }
        _ => {}
    }

    DefSubclassProc(hwnd, message, wparam, lparam)
}

unsafe fn remember_tree_label_edit_cancel(parent: HWND) {
    if parent.is_null() {
        return;
    }
    let Some(mut state) = window_state(parent) else {
        return;
    };
    if let Some(edit) = state.label_edit.as_mut() {
        edit.mark_canceled();
    }
}

unsafe fn remember_tree_label_edit_focus_loss_text(edit_control: HWND, parent: HWND) {
    if parent.is_null() {
        return;
    }

    let title = match window_text_utf8(edit_control, "tree label edit") {
        Ok(title) => title,
        Err(error) => {
            show_app_error(parent, &error);
            return;
        }
    };

    let Some(mut state) = window_state(parent) else {
        return;
    };
    if let Some(edit) = state.label_edit.as_mut() {
        edit.remember_focus_loss_title(title);
    }
}

fn resolve_label_edit_commit(
    label_edit: Option<TreeLabelEditState>,
    notification_title: Option<String>,
    fallback_node_id: Option<i64>,
) -> Option<(i64, String)> {
    match label_edit {
        Some(edit) => edit.commit_title(notification_title),
        None => {
            fallback_node_id.and_then(|node_id| notification_title.map(|title| (node_id, title)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_refinement_rejects_non_search_result_nodes() {
        let document =
            UiDocument::from_ui_nodes(vec![test_ui_node(DocumentTabSource::ActiveTree, "body")]);

        assert!(!can_refine_search_document_from_visible_results(
            &document,
            TreeMode::Active,
            "body",
            "body followup"
        ));
    }

    #[test]
    fn search_refinement_allows_search_result_with_title_match() {
        let document =
            UiDocument::from_ui_nodes(vec![test_ui_node(DocumentTabSource::SearchResult, "body")]);

        assert!(can_refine_search_document_from_visible_results(
            &document,
            TreeMode::Active,
            "body",
            "body followup"
        ));
    }

    #[test]
    fn search_refinement_rejects_short_to_trigram_query_transition() {
        let document =
            UiDocument::from_ui_nodes(vec![test_ui_node(DocumentTabSource::SearchResult, "abc")]);

        assert!(!can_refine_search_document_from_visible_results(
            &document,
            TreeMode::Active,
            "ab",
            "abc"
        ));
    }

    #[test]
    fn search_refinement_allows_short_title_only_refinement() {
        let document = UiDocument::from_ui_nodes(vec![test_ui_node(
            DocumentTabSource::SearchResult,
            "xy title",
        )]);

        assert!(can_refine_search_document_from_visible_results(
            &document,
            TreeMode::Active,
            "x",
            "xy"
        ));
    }

    #[test]
    fn search_refinement_rejects_content_matched_search_results() {
        let document =
            UiDocument::from_ui_nodes(vec![test_content_matched_search_ui_node("Notes")]);

        assert!(!can_refine_search_document_from_visible_results(
            &document,
            TreeMode::Active,
            "body",
            "body followup"
        ));
    }

    #[test]
    fn search_refinement_match_uses_literal_ascii_case_insensitive_contains() {
        assert!(contains_sqlite_like_literal("Release Plan", "release"));
        assert!(contains_sqlite_like_literal("Budget 100%", "100%"));
        assert!(contains_sqlite_like_literal("A_B Draft", "A_B"));
        assert!(contains_sqlite_like_literal("Use ^ caret", "^"));
        assert!(!contains_sqlite_like_literal("Budget 1000", "100%"));
    }

    #[test]
    fn search_refinement_match_preserves_unicode_literal_matching() {
        assert!(contains_sqlite_like_literal("한글 문서", "한글"));
        assert!(contains_sqlite_like_literal("math: ∑ café", "∑ café"));
        assert!(!contains_sqlite_like_literal("한글 문서", "영문"));
    }

    #[test]
    fn committed_label_edit_rejects_treeview_side_text_write() {
        assert_eq!(end_label_edit_reject_control_text(), 0);
    }

    #[test]
    fn programmatic_tree_selection_requires_navigation_save_for_different_active_tab() {
        assert!(programmatic_tree_selection_changes_active_tab(Some(1), 2));
    }

    #[test]
    fn programmatic_tree_selection_keeps_current_active_tab_without_navigation_save() {
        assert!(!programmatic_tree_selection_changes_active_tab(Some(1), 1));
        assert!(!programmatic_tree_selection_changes_active_tab(None, 1));
    }

    #[test]
    fn child_indices_map_supports_insert_position_for_sorted_siblings() {
        let document = UiDocument::from_ui_nodes(vec![
            test_display_node(1, None, "Beta"),
            test_display_node(2, None, "Alpha"),
            test_display_node(3, Some(1), "Child"),
        ]);
        let child_indices_by_parent = display_child_indices_by_parent(&document);
        let root_children: &[usize] = child_indices_by_parent
            .get(&None)
            .map_or(&[], Vec::as_slice);
        let root_ids: Vec<_> = root_children
            .iter()
            .map(|&index| document.nodes[index].id)
            .collect();

        assert_eq!(root_ids, vec![2, 1]);
        assert_eq!(
            previous_sibling_node_id(&document, root_children, 1),
            Some(2)
        );
        assert_eq!(previous_sibling_node_id(&document, root_children, 2), None);
    }

    #[test]
    fn visible_tree_items_match_allows_timestamp_only_changes() {
        let current = UiDocument::from_ui_nodes(vec![test_display_node(1, None, "Alpha")]);
        let mut changed_node = test_display_node(1, None, "Alpha");
        changed_node.updated_at = "2026-05-21T00:00:01Z".to_owned();
        let next = UiDocument::from_ui_nodes(vec![changed_node]);

        assert!(visible_tree_items_match(&current, &next));
    }

    #[test]
    fn visible_tree_items_match_rejects_reordered_nodes() {
        let current = UiDocument::from_ui_nodes(vec![
            test_display_node(1, None, "Alpha"),
            test_display_node(2, None, "Beta"),
        ]);
        let next = UiDocument::from_ui_nodes(vec![
            test_display_node(2, None, "Beta"),
            test_display_node(1, None, "Alpha"),
        ]);

        assert!(!visible_tree_items_match(&current, &next));
    }

    #[test]
    fn visible_tree_items_match_rejects_display_parent_changes() {
        let current = UiDocument::from_ui_nodes(vec![
            test_display_node(1, None, "Alpha"),
            test_display_node(2, None, "Beta"),
        ]);
        let next = UiDocument::from_ui_nodes(vec![
            test_display_node(1, Some(2), "Alpha"),
            test_display_node(2, None, "Beta"),
        ]);

        assert!(!visible_tree_items_match(&current, &next));
    }

    #[test]
    fn label_edit_commit_uses_focus_loss_title_when_notification_text_is_missing() {
        let mut edit = TreeLabelEditState::new(9);
        edit.remember_focus_loss_title("Blurred".to_owned());

        assert_eq!(
            resolve_label_edit_commit(Some(edit), None, Some(10)),
            Some((9, "Blurred".to_owned()))
        );
    }

    #[test]
    fn label_edit_commit_preserves_explicit_cancel() {
        let mut edit = TreeLabelEditState::new(9);
        edit.remember_focus_loss_title("Blurred".to_owned());
        edit.mark_canceled();

        assert_eq!(resolve_label_edit_commit(Some(edit), None, Some(10)), None);
    }

    #[test]
    fn label_edit_commit_falls_back_to_notification_node_without_session_state() {
        assert_eq!(
            resolve_label_edit_commit(None, Some("Renamed".to_owned()), Some(10)),
            Some((10, "Renamed".to_owned()))
        );
    }

    fn test_ui_node(source: DocumentTabSource, title: &str) -> UiNode {
        UiNode {
            id: 1,
            parent_id: None,
            display_parent_id: None,
            title: title.to_owned(),
            sort_order: 0,
            title_sort_key: title.to_owned(),
            display_title: Vec::new(),
            search_content_matched: false,
            updated_at: "2026-05-21T00:00:00Z".to_owned(),
            editable: true,
            source,
        }
    }

    fn test_display_node(id: i64, display_parent_id: Option<i64>, title: &str) -> UiNode {
        let mut node = test_ui_node(DocumentTabSource::ActiveTree, title);
        node.id = id;
        node.parent_id = display_parent_id;
        node.display_parent_id = display_parent_id;
        node.title_sort_key = title.to_owned();
        node
    }

    fn test_content_matched_search_ui_node(title: &str) -> UiNode {
        UiNode {
            search_content_matched: true,
            ..test_ui_node(DocumentTabSource::SearchResult, title)
        }
    }
}
