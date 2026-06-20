use std::borrow::Cow;
use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::ptr;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Controls::Dialogs::FINDREPLACEW;
use windows_sys::Win32::UI::Controls::HTREEITEM;
use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW;

use super::common::last_win32_error;
use super::dpi::{DpiMetrics, UiScale};
use super::font::EditorFontHandle;
use super::i18n::ui_text;
use super::size_move::SizeMoveState;
use super::text::{
    clear_editor_find_match_highlight, document_editor_plain_text_utf8_reusing,
    editor_caret_line_column, editor_view_state, highlight_editor_find_match_utf16,
    prepared_document_editor_selection_offset_count, restore_editor_view_state,
    set_editor_empty_read_only, set_editor_for_normalized_document_reusing,
    set_editor_for_prepared_document, utf8_to_wide_null, ProgrammaticTextUpdateGuard,
};
use super::theme::{rich_edit_find_highlight_colors, ThemeResources};
use crate::app::App;
use crate::domain::{
    Document, DocumentSearchResult, DocumentTabSource, DomainError, LoadedTabMetadataUpdate, Node,
    OpenDocumentTabInput, OpenTabs, UiLanguage, UiSettings, ROOT_NODE_ID,
};
use crate::error::AppError;
use crate::platform::gui::command_contract::{
    GuiCommandAvailability, GuiEditorAvailability, GuiTreeMode,
};

type TabContentByNodeId = HashMap<i64, (String, String)>;
pub(super) type TreeItemHandlesByNodeId = HashMap<i64, HTREEITEM>;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TreeMode {
    Active,
    Trash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveDocumentContentSync {
    LoadAll,
    SkipCurrent,
}

impl ActiveDocumentContentSync {
    fn should_load(self, loaded_updated_at: &str, current_updated_at: &str) -> bool {
        match self {
            Self::LoadAll => true,
            Self::SkipCurrent => loaded_updated_at != current_updated_at,
        }
    }
}

pub(super) type MenuState = GuiCommandAvailability;
pub(super) type EditorMenuState = GuiEditorAvailability;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FindMatchHighlight {
    start_utf16: usize,
    end_utf16: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct EditorImeCompositionState {
    active: bool,
    changed: bool,
}

impl EditorImeCompositionState {
    fn start(&mut self) {
        self.active = true;
        self.changed = false;
    }

    fn mark_changed(&mut self) {
        if self.active {
            self.changed = true;
        }
    }

    fn finish(&mut self) -> bool {
        let changed = self.active && self.changed;
        self.active = false;
        self.changed = false;
        changed
    }

    fn reset(&mut self) {
        self.active = false;
        self.changed = false;
    }

    fn is_active(self) -> bool {
        self.active
    }
}

impl From<TreeMode> for GuiTreeMode {
    fn from(value: TreeMode) -> Self {
        match value {
            TreeMode::Active => Self::Active,
            TreeMode::Trash => Self::Trash,
        }
    }
}

pub(super) struct WindowState {
    pub(super) app: App,
    pub(super) search: HWND,
    pub(super) tree: HWND,
    pub(super) tab_bar: HWND,
    pub(super) editor: HWND,
    pub(super) caret_status: HWND,
    pub(super) editor_font_handle: Option<EditorFontHandle>,
    pub(super) theme_resources: ThemeResources,
    pub(super) replace_dialog: Option<ReplaceDialogState>,
    pub(super) document: UiDocument,
    tree_item_handles_by_node_id: TreeItemHandlesByNodeId,
    pub(super) tabs: OpenTabs,
    pub(super) tree_mode: TreeMode,
    pub(super) search_query: String,
    pub(super) selected_node_id: Option<i64>,
    pub(super) label_edit: Option<TreeLabelEditState>,
    pub(super) pending_tree_label_edit_refresh_node_id: Option<i64>,
    pub(super) dragging_node_id: Option<i64>,
    pub(super) drag_highlight: Option<HTREEITEM>,
    pub(super) editor_text_loaded: bool,
    pub(super) editor_content_pending_sync: bool,
    pub(super) editor_text_utf16_buffer: Vec<u16>,
    pub(super) editor_text_utf8_buffer: String,
    current_find_match_highlight: Option<FindMatchHighlight>,
    editor_ime_composition: EditorImeCompositionState,
    pub(super) suppress_editor_change: Cell<bool>,
    pub(super) suppress_search_change: Cell<bool>,
    pub(super) search_debounce_timer_active: bool,
    pub(super) suppress_selection_change: bool,
    pub(super) suppress_tab_change: bool,
    pub(super) split_width: i32,
    pub(super) dragging_splitter: bool,
    pub(super) dpi_metrics: DpiMetrics,
    pub(super) ui_scale: UiScale,
    pub(super) size_move: SizeMoveState,
    pub(super) drop_on_destroy: bool,
}

impl WindowState {
    pub(super) fn new(
        app: App,
        document: UiDocument,
        ui_settings: UiSettings,
        dpi_metrics: DpiMetrics,
    ) -> Result<Self, AppError> {
        let theme_resources = ThemeResources::new(ui_settings.appearance.theme)?;
        let ui_scale = dpi_metrics.ui_scale();

        Ok(Self {
            app,
            search: ptr::null_mut(),
            tree: ptr::null_mut(),
            tab_bar: ptr::null_mut(),
            editor: ptr::null_mut(),
            caret_status: ptr::null_mut(),
            editor_font_handle: None,
            theme_resources,
            replace_dialog: None,
            document,
            tree_item_handles_by_node_id: TreeItemHandlesByNodeId::new(),
            tabs: OpenTabs::new(),
            tree_mode: TreeMode::Active,
            search_query: String::new(),
            selected_node_id: None,
            label_edit: None,
            pending_tree_label_edit_refresh_node_id: None,
            dragging_node_id: None,
            drag_highlight: None,
            editor_text_loaded: false,
            editor_content_pending_sync: false,
            editor_text_utf16_buffer: Vec::new(),
            editor_text_utf8_buffer: String::new(),
            current_find_match_highlight: None,
            editor_ime_composition: EditorImeCompositionState::default(),
            suppress_editor_change: Cell::new(false),
            suppress_search_change: Cell::new(false),
            search_debounce_timer_active: false,
            suppress_selection_change: false,
            suppress_tab_change: false,
            split_width: ui_settings.splitter.left_width,
            dragging_splitter: false,
            dpi_metrics,
            ui_scale,
            size_move: SizeMoveState::default(),
            drop_on_destroy: false,
        })
    }

    pub(super) fn set_dpi_metrics(&mut self, metrics: DpiMetrics) {
        self.dpi_metrics = metrics;
        self.ui_scale = metrics.ui_scale();
    }

    pub(super) fn selected_node(&self) -> Option<&UiNode> {
        self.selected_node_id
            .and_then(|node_id| self.document.node_by_id(node_id))
    }

    pub(super) fn replace_tree_item_handles(&mut self, handles: TreeItemHandlesByNodeId) {
        self.tree_item_handles_by_node_id = handles;
    }

    pub(super) fn remember_tree_item_handle(&mut self, node_id: i64, handle: HTREEITEM) {
        self.tree_item_handles_by_node_id.insert(node_id, handle);
    }

    pub(super) fn extend_tree_item_handles(&mut self, handles: TreeItemHandlesByNodeId) {
        self.tree_item_handles_by_node_id.extend(handles);
    }

    pub(super) fn forget_tree_item_handles(&mut self, node_ids: &[i64]) {
        for node_id in node_ids {
            self.tree_item_handles_by_node_id.remove(node_id);
        }
    }

    pub(super) fn clear_tree_item_handles(&mut self) {
        self.tree_item_handles_by_node_id.clear();
    }

    pub(super) fn tree_item_handle_by_node_id(&self, node_id: i64) -> Option<HTREEITEM> {
        self.tree_item_handles_by_node_id.get(&node_id).copied()
    }

    pub(super) fn menu_state(&self) -> MenuState {
        let selected_node_id = self.selected_node().map(|node| node.id);
        let search_active =
            self.tree_mode == TreeMode::Active && !self.search_query.trim().is_empty();
        let (move_up_enabled, move_down_enabled) =
            if self.tree_mode == TreeMode::Active && !search_active {
                sibling_move_availability(&self.document, selected_node_id)
            } else {
                (false, false)
            };

        MenuState::for_context(
            self.tree_mode.into(),
            search_active,
            selected_node_id,
            move_up_enabled,
            move_down_enabled,
            self.tabs.has_active(),
        )
    }

    pub(super) unsafe fn store_editor_content_in_active_tab(&mut self) -> Result<(), AppError> {
        let _ = self.store_editor_content_in_active_tab_and_get()?;
        Ok(())
    }

    pub(super) unsafe fn store_editor_content_in_active_tab_and_get(
        &mut self,
    ) -> Result<Option<&str>, AppError> {
        if self.suppress_editor_change.get() || self.tabs.active().is_none() {
            return Ok(None);
        }
        if !self.editor_text_loaded {
            return Err(AppError::platform(
                "store editor content",
                "editor text is not loaded for the active tab",
            ));
        }

        self.clear_current_find_match_highlight()?;
        let view_state = editor_view_state(self.editor);
        if self.editor_content_pending_sync {
            document_editor_plain_text_utf8_reusing(
                self.editor,
                &mut self.editor_text_utf16_buffer,
                &mut self.editor_text_utf8_buffer,
            )?;
            self.tabs
                .update_active_content_reusing(&mut self.editor_text_utf8_buffer);
            self.editor_content_pending_sync = false;
        }
        self.tabs.update_active_view_state(view_state);
        Ok(self.tabs.active().map(|tab| tab.content.as_str()))
    }

    pub(super) fn start_editor_ime_composition(&mut self) {
        self.editor_ime_composition.start();
    }

    pub(super) fn editor_ime_composition_active(&self) -> bool {
        self.editor_ime_composition.is_active()
    }

    pub(super) fn mark_editor_ime_composition_changed(&mut self) {
        self.editor_ime_composition.mark_changed();
    }

    pub(super) unsafe fn finish_editor_ime_composition(&mut self) -> Result<(), AppError> {
        if self.editor_ime_composition.finish() {
            self.clear_current_find_match_highlight()?;
            self.update_caret_status_from_editor()?;
        }
        Ok(())
    }

    pub(super) fn mark_editor_content_pending_from_view(&mut self) -> Result<bool, AppError> {
        if self.suppress_editor_change.get() || self.tabs.active().is_none() {
            return Ok(false);
        }
        if !self.editor_text_loaded {
            return Err(AppError::platform(
                "mark editor content dirty",
                "editor text is not loaded for the active tab",
            ));
        }
        if !self.tabs.active().is_some_and(|tab| tab.editable) {
            return Ok(false);
        }

        self.editor_content_pending_sync = true;
        Ok(self.tabs.mark_active_dirty_from_view())
    }

    pub(super) unsafe fn open_or_activate_tab_from_node(
        &mut self,
        node_index: usize,
    ) -> Result<(), AppError> {
        let Some(input) = self.tab_input_from_ui_node_index(node_index)? else {
            if !self.tabs.has_active() {
                self.show_active_tab_in_editor()?;
            }
            return Ok(());
        };

        self.store_editor_content_in_active_tab()?;
        self.tabs.open_or_activate(input);
        self.show_active_tab_in_editor()
    }

    pub(super) unsafe fn show_active_tab_in_editor(&mut self) -> Result<(), AppError> {
        self.editor_text_loaded = false;
        self.current_find_match_highlight = None;
        self.editor_ime_composition.reset();

        let editor_selection_offset_count = {
            let _guard = ProgrammaticTextUpdateGuard::enter(&self.suppress_editor_change);
            let editor_text_utf16_buffer = &mut self.editor_text_utf16_buffer;
            match self.tabs.active_mut() {
                Some(tab) => {
                    let was_dirty = tab.dirty;
                    let read_only = !tab.editable;
                    let result = set_editor_for_normalized_document_reusing(
                        self.editor,
                        Some(&mut tab.content),
                        read_only,
                        editor_text_utf16_buffer,
                    );
                    if !was_dirty {
                        tab.dirty = false;
                    }
                    result
                }
                None => {
                    set_editor_empty_read_only(self.editor)?;
                    Ok(0)
                }
            }
        }?;
        let restore_result = {
            let _guard = ProgrammaticTextUpdateGuard::enter(&self.suppress_editor_change);
            match self.tabs.active() {
                Some(tab) => restore_editor_view_state(
                    self.editor,
                    tab.view_state,
                    editor_selection_offset_count,
                ),
                None => Ok(()),
            }
        };
        restore_result?;
        self.editor_text_loaded = true;
        self.editor_content_pending_sync = false;
        self.update_caret_status_from_editor()?;
        Ok(())
    }

    pub(super) unsafe fn show_active_tab_in_editor_with_prepared_text(
        &mut self,
    ) -> Result<(), AppError> {
        self.editor_text_loaded = false;
        self.current_find_match_highlight = None;
        self.editor_ime_composition.reset();

        let editor_selection_offset_count = match self.tabs.active() {
            Some(_) => {
                prepared_document_editor_selection_offset_count(&self.editor_text_utf16_buffer)
            }
            None => 0,
        };
        let result = {
            let _guard = ProgrammaticTextUpdateGuard::enter(&self.suppress_editor_change);
            let editor_text_utf16_buffer = self.editor_text_utf16_buffer.as_slice();
            match self.tabs.active() {
                Some(tab) => set_editor_for_prepared_document(
                    self.editor,
                    editor_text_utf16_buffer,
                    !tab.editable,
                ),
                None => set_editor_empty_read_only(self.editor),
            }
        };
        result?;
        let restore_result = {
            let _guard = ProgrammaticTextUpdateGuard::enter(&self.suppress_editor_change);
            match self.tabs.active() {
                Some(tab) => restore_editor_view_state(
                    self.editor,
                    tab.view_state,
                    editor_selection_offset_count,
                ),
                None => Ok(()),
            }
        };
        restore_result?;
        self.editor_text_loaded = true;
        self.editor_content_pending_sync = false;
        self.update_caret_status_from_editor()?;
        Ok(())
    }

    pub(super) unsafe fn show_current_find_match_in_editor(
        &mut self,
        start_utf16: usize,
        end_utf16: usize,
    ) -> Result<(), AppError> {
        if start_utf16 >= end_utf16 || self.editor.is_null() {
            self.clear_current_find_match_highlight()?;
            self.current_find_match_highlight = None;
            return Ok(());
        }

        self.clear_current_find_match_highlight()?;
        let colors = rich_edit_find_highlight_colors(self.app.ui_settings().appearance.theme);
        let result = {
            let _guard = ProgrammaticTextUpdateGuard::enter(&self.suppress_editor_change);
            highlight_editor_find_match_utf16(self.editor, start_utf16, end_utf16, colors)
        };

        result?;
        self.current_find_match_highlight = Some(FindMatchHighlight {
            start_utf16,
            end_utf16,
        });
        self.update_caret_status_from_editor()
    }

    pub(super) unsafe fn clear_current_find_match_highlight(&mut self) -> Result<(), AppError> {
        let Some(highlight) = self.current_find_match_highlight else {
            return Ok(());
        };

        let result = {
            let _guard = ProgrammaticTextUpdateGuard::enter(&self.suppress_editor_change);
            clear_editor_find_match_highlight(
                self.editor,
                highlight.start_utf16,
                highlight.end_utf16,
            )
        };

        result?;
        self.current_find_match_highlight = None;
        Ok(())
    }

    pub(super) fn current_find_match_highlight_range(&self) -> Option<(usize, usize)> {
        self.current_find_match_highlight
            .map(|highlight| (highlight.start_utf16, highlight.end_utf16))
    }

    pub(super) unsafe fn update_caret_status_from_editor(&mut self) -> Result<(), AppError> {
        if self.caret_status.is_null() {
            return Ok(());
        }

        let position = editor_caret_line_column(self.editor).unwrap_or_default();
        let text = ui_text(self.app.ui_settings().language)
            .caret_position_status(position.line, position.column);
        let text = utf8_to_wide_null("convert caret status text", &text)?;
        if SetWindowTextW(self.caret_status, text.as_ptr()) == 0 {
            return Err(last_win32_error("set caret status text"));
        }

        Ok(())
    }

    pub(super) fn sync_tabs_from_active_document_local_metadata(
        &mut self,
        update_dirty_token: bool,
    ) -> Result<bool, AppError> {
        let tab_sync_targets: Vec<(usize, i64, bool)> = self
            .tabs
            .tabs()
            .iter()
            .enumerate()
            .map(|(index, tab)| (index, tab.node_id, tab.dirty))
            .collect();
        let dirty_tab_node_ids: Vec<i64> = if update_dirty_token {
            tab_sync_targets
                .iter()
                .filter_map(|(_, node_id, dirty)| dirty.then_some(*node_id))
                .collect()
        } else {
            Vec::new()
        };
        let current_contents = self.load_active_tab_contents_if_present(&dirty_tab_node_ids)?;
        let mut active_tab_node_ids = Vec::with_capacity(tab_sync_targets.len());

        for (index, node_id, was_dirty) in tab_sync_targets {
            let Some((node_id, parent_id, title, node_updated_at)) = self
                .app
                .document()
                .node_by_id(node_id)
                .filter(|node| node.deleted_at.is_none())
                .map(|node| {
                    (
                        node.id,
                        node.parent_id,
                        node.title.clone(),
                        node.updated_at.clone(),
                    )
                })
            else {
                continue;
            };

            let current_content = if update_dirty_token && was_dirty {
                current_contents.get(&node_id)
            } else {
                None
            };
            let current_content_for_dirty_token =
                current_content.map(|(content, _)| content.as_str());
            let loaded_updated_at =
                current_content.map_or(node_updated_at, |(_, updated_at)| updated_at.clone());

            active_tab_node_ids.push(node_id);
            self.tabs.sync_loaded_tab_metadata_preserving_content_at(
                index,
                LoadedTabMetadataUpdate {
                    node_id,
                    parent_id,
                    title,
                    loaded_updated_at,
                    editable: true,
                    source: DocumentTabSource::ActiveTree,
                    current_content_for_dirty_token,
                },
            );
        }

        Ok(self
            .tabs
            .mark_tabs_missing_from_active_document_read_only(&active_tab_node_ids))
    }

    pub(super) fn sync_tabs_from_active_document_metadata(
        &mut self,
        update_dirty_token: bool,
    ) -> Result<bool, AppError> {
        self.sync_tabs_from_active_document_metadata_with_content_sync(
            update_dirty_token,
            ActiveDocumentContentSync::LoadAll,
        )
    }

    pub(super) fn sync_tabs_from_reloaded_active_document_metadata(
        &mut self,
        update_dirty_token: bool,
    ) -> Result<bool, AppError> {
        self.sync_tabs_from_active_document_metadata_with_content_sync(
            update_dirty_token,
            ActiveDocumentContentSync::SkipCurrent,
        )
    }

    fn sync_tabs_from_active_document_metadata_with_content_sync(
        &mut self,
        update_dirty_token: bool,
        content_sync: ActiveDocumentContentSync,
    ) -> Result<bool, AppError> {
        let tab_sync_targets: Vec<(usize, i64, String)> = self
            .tabs
            .tabs()
            .iter()
            .enumerate()
            .map(|(index, tab)| (index, tab.node_id, tab.loaded_updated_at.clone()))
            .collect();
        let mut tab_metadata = Vec::with_capacity(tab_sync_targets.len());

        for (index, node_id, loaded_updated_at) in tab_sync_targets {
            let Some((node_id, parent_id, title, updated_at)) = self
                .app
                .document()
                .node_by_id(node_id)
                .filter(|node| node.deleted_at.is_none())
                .map(|node| {
                    (
                        node.id,
                        node.parent_id,
                        node.title.clone(),
                        node.updated_at.clone(),
                    )
                })
            else {
                continue;
            };
            tab_metadata.push((
                index,
                node_id,
                parent_id,
                title,
                updated_at,
                loaded_updated_at,
            ));
        }

        let content_node_ids: Vec<i64> = tab_metadata
            .iter()
            .filter_map(|(_, node_id, _, _, updated_at, loaded_updated_at)| {
                content_sync
                    .should_load(loaded_updated_at, updated_at)
                    .then_some(*node_id)
            })
            .collect();
        let mut contents = self.load_active_tab_contents_if_present(&content_node_ids)?;
        let mut active_tab_node_ids = Vec::with_capacity(tab_metadata.len());

        for (index, node_id, parent_id, title, updated_at, loaded_updated_at) in tab_metadata {
            if !content_sync.should_load(&loaded_updated_at, &updated_at) {
                active_tab_node_ids.push(node_id);
                self.tabs.sync_loaded_tab_metadata_preserving_content_at(
                    index,
                    LoadedTabMetadataUpdate {
                        node_id,
                        parent_id,
                        title,
                        loaded_updated_at: updated_at,
                        editable: true,
                        source: DocumentTabSource::ActiveTree,
                        current_content_for_dirty_token: None,
                    },
                );
                continue;
            }

            let Some((content, updated_at)) = contents.remove(&node_id) else {
                continue;
            };
            let input = OpenDocumentTabInput {
                node_id,
                parent_id,
                title,
                content,
                loaded_updated_at: updated_at,
                editable: true,
                source: DocumentTabSource::ActiveTree,
            };

            active_tab_node_ids.push(input.node_id);
            self.tabs.sync_loaded_tab(input, update_dirty_token);
        }

        Ok(self
            .tabs
            .mark_tabs_missing_from_active_document_read_only(&active_tab_node_ids))
    }

    pub(super) fn sync_tabs_from_visible_document(
        &mut self,
        update_dirty_token: bool,
    ) -> Result<(), AppError> {
        let tab_sync_targets: Vec<(i64, String)> = self
            .tabs
            .tabs()
            .iter()
            .map(|tab| (tab.node_id, tab.loaded_updated_at.clone()))
            .collect();

        let mut active_content_node_ids = Vec::new();
        let mut deleted_content_node_ids = Vec::new();
        for (node_id, loaded_updated_at) in &tab_sync_targets {
            let Some(node) = self.document.node_by_id(*node_id) else {
                continue;
            };
            if tab_content_is_current(loaded_updated_at, node) {
                continue;
            }

            match node.source {
                DocumentTabSource::ActiveTree | DocumentTabSource::SearchResult => {
                    active_content_node_ids.push(node.id);
                }
                DocumentTabSource::Trash => {
                    deleted_content_node_ids.push(node.id);
                }
            }
        }

        let (mut active_contents, mut deleted_contents) =
            self.load_visible_tab_contents(&active_content_node_ids, &deleted_content_node_ids)?;

        for (node_id, loaded_updated_at) in tab_sync_targets {
            let Some(node_index) = self.document.node_index_by_id(node_id) else {
                continue;
            };

            if tab_content_is_current(loaded_updated_at.as_str(), &self.document.nodes[node_index])
            {
                let node = &self.document.nodes[node_index];
                self.tabs.sync_loaded_tab_metadata(
                    node.id,
                    node.parent_id,
                    node.title.clone(),
                    node.editable,
                    node.source,
                );
                continue;
            }

            let Some(input) = self.tab_input_from_ui_node_index_with_contents(
                node_index,
                &mut active_contents,
                &mut deleted_contents,
            )?
            else {
                continue;
            };
            self.tabs.sync_loaded_tab(input, update_dirty_token);
        }

        Ok(())
    }

    pub(super) fn sync_tabs_from_visible_document_preserving_content(&mut self) {
        let tab_count = self.tabs.tabs().len();

        for index in 0..tab_count {
            let tab = &self.tabs.tabs()[index];
            let Some(node) = self.document.node_by_id(tab.node_id) else {
                continue;
            };
            if tab.parent_id == node.parent_id
                && tab.title == node.title
                && tab.loaded_updated_at == node.updated_at
                && tab.editable == node.editable
                && tab.source == node.source
            {
                continue;
            }

            self.tabs.sync_loaded_tab_metadata_preserving_content_at(
                index,
                LoadedTabMetadataUpdate {
                    node_id: node.id,
                    parent_id: node.parent_id,
                    title: node.title.clone(),
                    loaded_updated_at: node.updated_at.clone(),
                    editable: node.editable,
                    source: node.source,
                    current_content_for_dirty_token: None,
                },
            );
        }
    }

    pub(super) fn sync_tabs_from_visible_document_metadata_preserving_content(&mut self) {
        let tab_count = self.tabs.tabs().len();

        for index in 0..tab_count {
            let tab = &self.tabs.tabs()[index];
            let Some(node) = self.document.node_by_id(tab.node_id) else {
                continue;
            };
            if tab.parent_id == node.parent_id
                && tab.title == node.title
                && tab.editable == node.editable
                && tab.source == node.source
            {
                continue;
            }

            self.tabs.sync_loaded_tab_metadata_at(
                index,
                node.id,
                node.parent_id,
                node.title.clone(),
                node.editable,
                node.source,
            );
        }
    }

    fn load_active_tab_contents_if_present(
        &self,
        node_ids: &[i64],
    ) -> Result<TabContentByNodeId, AppError> {
        self.app.load_active_node_contents_if_present(node_ids)
    }

    fn load_visible_tab_contents(
        &self,
        active_node_ids: &[i64],
        deleted_node_ids: &[i64],
    ) -> Result<(TabContentByNodeId, TabContentByNodeId), AppError> {
        if active_node_ids.is_empty() && deleted_node_ids.is_empty() {
            return Ok((HashMap::new(), HashMap::new()));
        }

        let active_contents = self
            .app
            .load_active_node_contents_if_present(active_node_ids)?;
        let deleted_contents = self.app.load_deleted_node_contents(deleted_node_ids)?;

        for node_id in deleted_node_ids {
            if !deleted_contents.contains_key(node_id) {
                return Err(DomainError::NodeNotFound { node_id: *node_id }.into());
            }
        }

        Ok((active_contents, deleted_contents))
    }

    fn tab_input_from_ui_node_index(
        &self,
        node_index: usize,
    ) -> Result<Option<OpenDocumentTabInput>, AppError> {
        let Some(node) = self.document.nodes.get(node_index) else {
            return Ok(None);
        };

        match node.source {
            DocumentTabSource::ActiveTree => {
                let Some((content, updated_at)) =
                    self.app.load_active_node_content_if_present(node.id)?
                else {
                    return Ok(None);
                };
                Ok(Some(tab_input_from_ui_node(node, content, updated_at)))
            }
            DocumentTabSource::Trash => {
                let (content, updated_at) = self.app.load_deleted_node_content(node.id)?;
                Ok(Some(tab_input_from_ui_node(node, content, updated_at)))
            }
            DocumentTabSource::SearchResult => {
                let Some((content, updated_at)) =
                    self.app.load_active_node_content_if_present(node.id)?
                else {
                    return Ok(None);
                };
                Ok(Some(tab_input_from_ui_node(node, content, updated_at)))
            }
        }
    }

    fn tab_input_from_ui_node_index_with_contents(
        &self,
        node_index: usize,
        active_contents: &mut TabContentByNodeId,
        deleted_contents: &mut TabContentByNodeId,
    ) -> Result<Option<OpenDocumentTabInput>, AppError> {
        let Some(node) = self.document.nodes.get(node_index) else {
            return Ok(None);
        };

        match node.source {
            DocumentTabSource::ActiveTree | DocumentTabSource::SearchResult => {
                let Some((content, updated_at)) = active_contents.remove(&node.id) else {
                    return Ok(None);
                };
                Ok(Some(tab_input_from_ui_node(node, content, updated_at)))
            }
            DocumentTabSource::Trash => {
                let Some((content, updated_at)) = deleted_contents.remove(&node.id) else {
                    return Err(DomainError::NodeNotFound { node_id: node.id }.into());
                };
                Ok(Some(tab_input_from_ui_node(node, content, updated_at)))
            }
        }
    }
}

fn tab_content_is_current(loaded_updated_at: &str, node: &UiNode) -> bool {
    loaded_updated_at == node.updated_at.as_str()
}

fn sibling_move_availability(document: &UiDocument, selected_node_id: Option<i64>) -> (bool, bool) {
    let Some(selected_node_id) = selected_node_id else {
        return (false, false);
    };
    if selected_node_id == ROOT_NODE_ID {
        return (false, false);
    }

    let Some(selected_node) = document.node_by_id(selected_node_id) else {
        return (false, false);
    };

    if let Some(child_range) =
        document.display_ordered_child_index_range(selected_node.display_parent_id)
    {
        let sibling_candidates = &document.nodes[child_range];
        if sibling_candidates
            .iter()
            .any(|node| node.id == selected_node_id)
        {
            return sibling_move_availability_from_candidates(
                sibling_candidates,
                selected_node_id,
                selected_node,
            );
        }
    }

    sibling_move_availability_from_candidates(&document.nodes, selected_node_id, selected_node)
}

fn sibling_move_availability_from_candidates(
    candidates: &[UiNode],
    selected_node_id: i64,
    selected_node: &UiNode,
) -> (bool, bool) {
    let mut has_previous_sibling = false;
    let mut has_next_sibling = false;
    for node in candidates {
        if node.id == selected_node_id || node.display_parent_id != selected_node.display_parent_id
        {
            continue;
        }

        match compare_ui_nodes_for_display(node, selected_node) {
            Ordering::Less => has_previous_sibling = true,
            Ordering::Greater => has_next_sibling = true,
            Ordering::Equal => {}
        }

        if has_previous_sibling && has_next_sibling {
            break;
        }
    }

    (has_previous_sibling, has_next_sibling)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TreeLabelEditState {
    node_id: i64,
    focus_loss_title: Option<String>,
    canceled: bool,
}

impl TreeLabelEditState {
    pub(super) fn new(node_id: i64) -> Self {
        Self {
            node_id,
            focus_loss_title: None,
            canceled: false,
        }
    }

    pub(super) fn remember_focus_loss_title(&mut self, title: String) {
        if !self.canceled {
            self.focus_loss_title = Some(title);
        }
    }

    pub(super) fn mark_canceled(&mut self) {
        self.canceled = true;
        self.focus_loss_title = None;
    }

    pub(super) fn commit_title(self, notification_title: Option<String>) -> Option<(i64, String)> {
        if self.canceled {
            return None;
        }

        notification_title
            .or(self.focus_loss_title)
            .map(|title| (self.node_id, title))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FindReplaceDialogKind {
    Find,
    Replace,
}

pub(super) struct ReplaceDialogState {
    pub(super) kind: FindReplaceDialogKind,
    pub(super) hwnd: HWND,
    pub(super) _find_buffer: Vec<u16>,
    pub(super) _replace_buffer: Vec<u16>,
    pub(super) request: Box<FINDREPLACEW>,
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::domain::DomainError;
    use crate::infra::sqlite::SqliteDocumentRepository;

    #[test]
    fn editor_ime_composition_state_coalesces_changes_until_end() {
        let mut state = EditorImeCompositionState::default();

        state.start();
        assert!(state.is_active());
        state.mark_changed();
        state.mark_changed();

        assert!(state.finish());
        assert!(!state.is_active());
        assert!(!state.finish());
    }

    #[test]
    fn editor_ime_composition_state_ignores_changes_outside_composition() {
        let mut state = EditorImeCompositionState::default();

        state.mark_changed();
        assert!(!state.finish());

        state.start();
        assert!(!state.finish());
    }

    #[test]
    fn active_tree_browse_menu_state_tracks_selection_and_root() {
        assert_eq!(
            MenuState::for_context(GuiTreeMode::Active, false, None, false, false, false),
            MenuState {
                save_enabled: false,
                close_tab_enabled: false,
                new_child_document_enabled: false,
                new_document_enabled: true,
                rename_enabled: false,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked: true,
                trash_checked: false,
            }
        );

        assert_eq!(
            MenuState::for_context(
                GuiTreeMode::Active,
                false,
                Some(ROOT_NODE_ID),
                false,
                false,
                true
            ),
            MenuState {
                save_enabled: true,
                close_tab_enabled: true,
                new_child_document_enabled: true,
                new_document_enabled: true,
                rename_enabled: true,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked: true,
                trash_checked: false,
            }
        );

        assert_eq!(
            MenuState::for_context(GuiTreeMode::Active, false, Some(2), true, true, true),
            MenuState {
                save_enabled: true,
                close_tab_enabled: true,
                new_child_document_enabled: true,
                new_document_enabled: true,
                rename_enabled: true,
                move_up_enabled: true,
                move_down_enabled: true,
                delete_enabled: true,
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked: true,
                trash_checked: false,
            }
        );
    }

    #[test]
    fn search_results_disable_node_commands_and_keep_active_tree_checked() {
        assert_eq!(
            MenuState::for_context(GuiTreeMode::Active, true, Some(2), true, true, true),
            MenuState {
                save_enabled: true,
                close_tab_enabled: true,
                new_child_document_enabled: false,
                new_document_enabled: false,
                rename_enabled: false,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked: true,
                trash_checked: false,
            }
        );
    }

    #[test]
    fn active_ui_document_builds_metadata_node_without_sort_key() -> Result<(), AppError> {
        let document = Document::new(vec![
            test_node(ROOT_NODE_ID, None, "Root", 0, ""),
            test_node(2, Some(ROOT_NODE_ID), "Draft", 0, "large body"),
        ])?;

        let ui_document = UiDocument::from_active_document(&document)?;
        let draft = ui_document
            .node_by_id(2)
            .ok_or(DomainError::NodeNotFound { node_id: 2 })?;

        assert!(draft.title_sort_key.is_empty());
        Ok(())
    }

    #[test]
    fn active_ui_document_trusts_source_order_without_resorting() -> Result<(), AppError> {
        let document = Document::new(vec![
            test_node(ROOT_NODE_ID, None, "Root", 0, ""),
            test_node(4, Some(ROOT_NODE_ID), "Gamma", 2, ""),
            test_node(2, Some(ROOT_NODE_ID), "Alpha", 0, ""),
            test_node(3, Some(ROOT_NODE_ID), "Beta", 1, ""),
        ])?;

        let ui_document = UiDocument::from_active_document(&document)?;

        assert_eq!(ui_node_ids(&ui_document), vec![ROOT_NODE_ID, 4, 2, 3]);
        Ok(())
    }

    #[test]
    fn ui_document_remove_nodes_updates_id_caches_for_remaining_nodes() -> Result<(), AppError> {
        let document = Document::new(vec![
            test_node(ROOT_NODE_ID, None, "Root", 0, ""),
            test_node(2, Some(ROOT_NODE_ID), "Alpha", 0, ""),
            test_node(3, Some(ROOT_NODE_ID), "Beta", 1, ""),
            test_node(4, Some(ROOT_NODE_ID), "Gamma", 2, ""),
            test_node(5, Some(ROOT_NODE_ID), "Delta", 3, ""),
        ])?;
        let mut ui_document = UiDocument::from_active_document(&document)?;

        ui_document.remove_nodes_by_id(&[2, 4]);

        assert_eq!(ui_node_ids(&ui_document), vec![ROOT_NODE_ID, 3, 5]);
        assert!(!ui_document.contains_node_id(2));
        assert!(!ui_document.contains_node_id(4));
        assert_eq!(ui_document.node_index_by_id(3), Some(1));
        assert_eq!(ui_document.node_index_by_id(5), Some(2));
        assert_eq!(ui_document.node_by_id(4).map(|node| node.id), None);
        Ok(())
    }

    #[test]
    fn move_sync_updates_only_display_ordered_sibling_range() -> Result<(), AppError> {
        let before = Document::new(vec![
            test_node(ROOT_NODE_ID, None, "Root", 0, ""),
            test_node(2, Some(ROOT_NODE_ID), "Alpha", 0, ""),
            test_node(3, Some(ROOT_NODE_ID), "Beta", 1, ""),
            test_node(4, Some(ROOT_NODE_ID), "Gamma", 2, ""),
        ])?;
        let mut ui_document = UiDocument::from_active_document(&before)?;
        let after = Document::new(vec![
            test_node(ROOT_NODE_ID, None, "Root", 0, ""),
            test_node(3, Some(ROOT_NODE_ID), "Beta", 0, ""),
            test_node(2, Some(ROOT_NODE_ID), "Alpha", 1, ""),
            test_node(4, Some(ROOT_NODE_ID), "Gamma", 2, ""),
        ])?;

        ui_document.sync_active_nodes_after_move(
            &after,
            3,
            Some(ROOT_NODE_ID),
            Some(ROOT_NODE_ID),
        )?;

        assert_eq!(ui_node_ids(&ui_document), vec![ROOT_NODE_ID, 3, 2, 4]);
        assert_eq!(
            ui_document.node_by_id(2).map(|node| node.sort_order),
            Some(1)
        );
        assert_eq!(ui_document.node_index_by_id(3), Some(1));
        assert_eq!(ui_document.node_index_by_id(2), Some(2));
        Ok(())
    }

    #[test]
    fn move_sync_reparents_node_without_resorting_whole_document() -> Result<(), AppError> {
        let before = Document::new(vec![
            test_node(ROOT_NODE_ID, None, "Root", 0, ""),
            test_node(2, Some(ROOT_NODE_ID), "Alpha", 0, ""),
            test_node(3, Some(ROOT_NODE_ID), "Beta", 1, ""),
            test_node(10, Some(ROOT_NODE_ID), "Folder", 2, ""),
            test_node(11, Some(10), "Child", 0, ""),
        ])?;
        let mut ui_document = UiDocument::from_active_document(&before)?;
        let after = Document::new(vec![
            test_node(ROOT_NODE_ID, None, "Root", 0, ""),
            test_node(2, Some(ROOT_NODE_ID), "Alpha", 0, ""),
            test_node(10, Some(ROOT_NODE_ID), "Folder", 1, ""),
            test_node(11, Some(10), "Child", 0, ""),
            test_node(3, Some(10), "Beta", 1, ""),
        ])?;

        ui_document.sync_active_nodes_after_move(&after, 3, Some(ROOT_NODE_ID), Some(10))?;

        assert_eq!(ui_node_ids(&ui_document), vec![ROOT_NODE_ID, 2, 10, 11, 3]);
        assert_eq!(
            ui_document.node_by_id(3).map(|node| node.parent_id),
            Some(Some(10))
        );
        assert_eq!(
            ui_document.node_by_id(10).map(|node| node.sort_order),
            Some(1)
        );
        assert_eq!(ui_document.node_index_by_id(11), Some(3));
        assert_eq!(ui_document.node_index_by_id(3), Some(4));
        Ok(())
    }

    #[test]
    fn search_result_ui_document_preserves_result_order() -> Result<(), AppError> {
        let results = vec![
            DocumentSearchResult {
                node: test_node(3, Some(ROOT_NODE_ID), "Beta", 1, ""),
                parent_title: Some("Root".to_owned()),
                content_matched: true,
            },
            DocumentSearchResult {
                node: test_node(2, Some(ROOT_NODE_ID), "Alpha", 0, ""),
                parent_title: Some("Root".to_owned()),
                content_matched: false,
            },
        ];

        let ui_document = UiDocument::from_search_results(results, UiLanguage::Korean)?;

        assert_eq!(ui_node_ids(&ui_document), vec![3, 2]);
        Ok(())
    }

    #[test]
    fn search_result_ui_document_omits_body_and_loads_tab_content() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "stored body")?;
            let draft_id = draft.id;
            let results = repository.search_documents("stored", 200)?;
            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let document = UiDocument::from_search_results(results, ui_settings.language)?;
            let state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            let node_index = state
                .document
                .nodes
                .iter()
                .position(|node| node.id == draft_id)
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            let search_node = &state.document.nodes[node_index];
            assert!(search_node.title_sort_key.is_empty());
            assert!(search_node.search_content_matched);

            let input = state
                .tab_input_from_ui_node_index(node_index)?
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(input.content.as_str(), "stored body");
            assert_eq!(input.source, DocumentTabSource::SearchResult);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn sync_tabs_from_active_document_keeps_loaded_body_for_metadata_document(
    ) -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "stored body")?;
            let draft_id = draft.id;
            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let document = UiDocument::from_active_document(app.document())?;
            let mut state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            let metadata_node = state
                .app
                .document()
                .node_by_id(draft_id)
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(metadata_node.content.as_str(), "");

            let node_index = state
                .document
                .nodes
                .iter()
                .position(|node| node.id == draft_id)
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            let input = state
                .tab_input_from_ui_node_index(node_index)?
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            state.tabs.open_or_activate(input);

            state.app.rename_node(draft_id, "Renamed")?;
            state.sync_tabs_from_active_document_metadata(true)?;

            let tab = state
                .tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(tab.title.as_str(), "Renamed");
            assert_eq!(tab.content.as_str(), "stored body");
            assert_eq!(tab.loaded_content(), "stored body");
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn sync_tabs_from_reloaded_active_document_metadata_skips_current_tab_body_reload(
    ) -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "stored body")?;
            let draft_id = draft.id;
            let draft_updated_at = draft.updated_at.clone();
            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let document = UiDocument::from_active_document(app.document())?;
            let mut state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            state.tabs.open_or_activate(OpenDocumentTabInput {
                node_id: draft_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Draft".to_owned(),
                content: "cached current body".to_owned(),
                loaded_updated_at: draft_updated_at.clone(),
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });
            let (content_ptr, loaded_content_ptr) = {
                let tab = state
                    .tabs
                    .active()
                    .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
                (tab.content.as_ptr(), tab.loaded_content().as_ptr())
            };

            state.app.reload_document()?;
            state.sync_tabs_from_reloaded_active_document_metadata(true)?;

            let tab = state
                .tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(tab.title.as_str(), "Draft");
            assert_eq!(tab.content.as_str(), "cached current body");
            assert_eq!(tab.content.as_ptr(), content_ptr);
            assert_eq!(tab.loaded_content(), "cached current body");
            assert_eq!(tab.loaded_content().as_ptr(), loaded_content_ptr);
            assert_eq!(tab.loaded_updated_at.as_str(), draft_updated_at.as_str());
            assert!(tab.editable);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn sync_tabs_from_active_document_marks_externally_deleted_tab_read_only(
    ) -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "stored body")?;
            let draft_id = draft.id;
            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let document = UiDocument::from_active_document(app.document())?;
            let mut state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            let node_index = state
                .document
                .nodes
                .iter()
                .position(|node| node.id == draft_id)
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            let input = state
                .tab_input_from_ui_node_index(node_index)?
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            state.tabs.open_or_activate(input);

            let mut external = SqliteDocumentRepository::open(&db_path)?;
            external.soft_delete_node_cascade(draft_id)?;

            assert!(state.sync_tabs_from_active_document_metadata(true)?);
            let tab = state
                .tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(tab.node_id, draft_id);
            assert!(!tab.editable);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn sync_tabs_from_visible_document_preserves_open_active_tab_content(
    ) -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "stored body")?;
            let draft_id = draft.id;
            let draft_updated_at = draft.updated_at.clone();
            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let document = UiDocument::from_active_document(app.document())?;
            let mut state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            state.tabs.open_or_activate(OpenDocumentTabInput {
                node_id: draft_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Draft".to_owned(),
                content: "loaded body".to_owned(),
                loaded_updated_at: draft_updated_at,
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });

            state.app.rename_node(draft_id, "Renamed")?;
            state.document = UiDocument::from_active_document(state.app.document())?;
            state.sync_tabs_from_visible_document_preserving_content();

            let tab = state
                .tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(tab.title.as_str(), "Renamed");
            assert_eq!(tab.content.as_str(), "loaded body");
            assert_eq!(tab.loaded_content(), "loaded body");
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn sync_tabs_from_visible_document_metadata_preserves_content_and_save_conflict_token(
    ) -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "stored body")?;
            let draft_id = draft.id;
            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let mut document = UiDocument::from_active_document(app.document())?;
            let node = document
                .nodes
                .iter_mut()
                .find(|node| node.id == draft_id)
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            node.title = "Search result title".to_owned();
            node.parent_id = None;
            node.updated_at = "search-token".to_owned();
            node.source = DocumentTabSource::SearchResult;
            let mut state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            state.tabs.open_or_activate(OpenDocumentTabInput {
                node_id: draft_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Draft".to_owned(),
                content: "cached open body".to_owned(),
                loaded_updated_at: "initial-token".to_owned(),
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });

            state.sync_tabs_from_visible_document_metadata_preserving_content();

            let tab = state
                .tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(tab.title.as_str(), "Search result title");
            assert_eq!(tab.parent_id, None);
            assert_eq!(tab.content.as_str(), "cached open body");
            assert_eq!(tab.loaded_content(), "cached open body");
            assert_eq!(tab.loaded_updated_at.as_str(), "initial-token");
            assert_eq!(tab.source, DocumentTabSource::SearchResult);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn load_visible_tab_contents_batches_active_and_deleted_content() -> Result<(), Box<dyn Error>>
    {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let active_one =
                repository.create_document_with_content(ROOT_NODE_ID, "Active 1", "active one")?;
            let active_two =
                repository.create_document_with_content(ROOT_NODE_ID, "Active 2", "active two")?;
            let deleted =
                repository.create_document_with_content(ROOT_NODE_ID, "Deleted", "deleted body")?;
            let deleted_id = deleted.id;
            repository.soft_delete_node_cascade(deleted_id)?;

            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let document = UiDocument::from_active_document(app.document())?;
            let state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            let (active_contents, deleted_contents) = state.load_visible_tab_contents(
                &[active_one.id, active_two.id, deleted_id],
                &[deleted_id],
            )?;

            assert_eq!(
                active_contents
                    .get(&active_one.id)
                    .map(|content| content.0.as_str()),
                Some("active one")
            );
            assert_eq!(
                active_contents
                    .get(&active_two.id)
                    .map(|content| content.0.as_str()),
                Some("active two")
            );
            assert!(!active_contents.contains_key(&deleted_id));
            assert_eq!(
                deleted_contents
                    .get(&deleted_id)
                    .map(|content| content.0.as_str()),
                Some("deleted body")
            );
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn sync_tabs_from_active_document_local_metadata_preserves_open_tab_content(
    ) -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "stored body")?;
            let draft_id = draft.id;
            let draft_updated_at = draft.updated_at.clone();
            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let document = UiDocument::from_active_document(app.document())?;
            let mut state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            state.tabs.open_or_activate(OpenDocumentTabInput {
                node_id: draft_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Draft".to_owned(),
                content: "loaded body".to_owned(),
                loaded_updated_at: draft_updated_at,
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });
            let content_ptr = state
                .tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?
                .content
                .as_ptr();

            state.app.rename_node(draft_id, "Renamed")?;
            state.sync_tabs_from_active_document_local_metadata(true)?;

            let tab = state
                .tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            let node = state
                .app
                .document()
                .node_by_id(draft_id)
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(tab.title.as_str(), "Renamed");
            assert_eq!(tab.content.as_str(), "loaded body");
            assert_eq!(tab.content.as_ptr(), content_ptr);
            assert_eq!(tab.loaded_content(), "loaded body");
            assert_eq!(tab.loaded_updated_at.as_str(), node.updated_at.as_str());
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn sync_tabs_from_active_document_local_metadata_advances_dirty_baseline_token(
    ) -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "stored body")?;
            let draft_id = draft.id;
            let draft_updated_at = draft.updated_at.clone();
            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let document = UiDocument::from_active_document(app.document())?;
            let mut state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            state.tabs.open_or_activate(OpenDocumentTabInput {
                node_id: draft_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Draft".to_owned(),
                content: "stored body".to_owned(),
                loaded_updated_at: draft_updated_at,
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });
            state.tabs.update_active_content("draft body".to_owned());

            state.app.rename_node(draft_id, "Renamed")?;
            state.sync_tabs_from_active_document_local_metadata(true)?;

            let tab = state
                .tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            let node = state
                .app
                .document()
                .node_by_id(draft_id)
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(tab.title.as_str(), "Renamed");
            assert_eq!(tab.content.as_str(), "draft body");
            assert_eq!(tab.loaded_content(), "stored body");
            assert_eq!(tab.loaded_updated_at.as_str(), node.updated_at.as_str());
            assert!(tab.dirty);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn sync_tabs_from_active_document_local_metadata_keeps_dirty_token_when_loaded_body_is_stale(
    ) -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "stored body")?;
            let draft_id = draft.id;
            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let document = UiDocument::from_active_document(app.document())?;
            let mut state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            state.tabs.open_or_activate(OpenDocumentTabInput {
                node_id: draft_id,
                parent_id: Some(ROOT_NODE_ID),
                title: "Draft".to_owned(),
                content: "stale loaded body".to_owned(),
                loaded_updated_at: "stale-token".to_owned(),
                editable: true,
                source: DocumentTabSource::ActiveTree,
            });
            state.tabs.update_active_content("draft body".to_owned());

            state.app.rename_node(draft_id, "Renamed")?;
            state.sync_tabs_from_active_document_local_metadata(true)?;

            let tab = state
                .tabs
                .active()
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            let node = state
                .app
                .document()
                .node_by_id(draft_id)
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(tab.title.as_str(), "Renamed");
            assert_eq!(tab.content.as_str(), "draft body");
            assert_eq!(tab.loaded_content(), "stale loaded body");
            assert_eq!(tab.loaded_updated_at.as_str(), "stale-token");
            assert_ne!(tab.loaded_updated_at.as_str(), node.updated_at.as_str());
            assert!(tab.dirty);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn search_result_tab_input_loads_body_from_repository() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "stored body")?;
            let draft_id = draft.id;
            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let document = UiDocument::from_ui_nodes(vec![UiNode {
                id: draft_id,
                parent_id: Some(ROOT_NODE_ID),
                display_parent_id: None,
                title: "Draft".to_owned(),
                sort_order: 0,
                title_sort_key: String::new(),
                display_title: Vec::new(),
                search_content_matched: false,
                updated_at: draft.updated_at,
                editable: true,
                source: DocumentTabSource::SearchResult,
            }]);
            let state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            let input = state
                .tab_input_from_ui_node_index(0)?
                .ok_or(DomainError::NodeNotFound { node_id: draft_id })?;
            assert_eq!(input.content.as_str(), "stored body");
            assert_eq!(input.source, DocumentTabSource::SearchResult);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn sibling_move_availability_tracks_order_boundaries() -> Result<(), AppError> {
        let document = Document::new(vec![
            test_node(ROOT_NODE_ID, None, "Root", 0, ""),
            test_node(2, Some(ROOT_NODE_ID), "Alpha", 0, ""),
            test_node(3, Some(ROOT_NODE_ID), "Beta", 1, ""),
            test_node(4, Some(ROOT_NODE_ID), "Gamma", 2, ""),
        ])?;
        let ui_document = UiDocument::from_active_document(&document)?;

        assert_eq!(
            sibling_move_availability(&ui_document, None),
            (false, false)
        );
        assert_eq!(
            sibling_move_availability(&ui_document, Some(ROOT_NODE_ID)),
            (false, false)
        );
        assert_eq!(
            sibling_move_availability(&ui_document, Some(2)),
            (false, true)
        );
        assert_eq!(
            sibling_move_availability(&ui_document, Some(3)),
            (true, true)
        );
        assert_eq!(
            sibling_move_availability(&ui_document, Some(4)),
            (true, false)
        );

        Ok(())
    }

    #[test]
    fn sibling_move_availability_uses_display_order_when_nodes_are_not_vector_sorted(
    ) -> Result<(), AppError> {
        let document = Document::new(vec![
            test_node(ROOT_NODE_ID, None, "Root", 0, ""),
            test_node(2, Some(ROOT_NODE_ID), "Alpha", 0, ""),
            test_node(3, Some(ROOT_NODE_ID), "Beta", 1, ""),
            test_node(4, Some(ROOT_NODE_ID), "Gamma", 2, ""),
        ])?;
        let mut ui_document = UiDocument::from_active_document(&document)?;
        ui_document.nodes.swap(1, 3);

        assert_eq!(
            sibling_move_availability(&ui_document, Some(2)),
            (false, true)
        );
        assert_eq!(
            sibling_move_availability(&ui_document, Some(4)),
            (true, false)
        );

        Ok(())
    }

    #[test]
    fn trash_ui_document_builds_metadata_node_without_sort_key() -> Result<(), AppError> {
        let deleted = Node {
            deleted_at: Some("2026-04-30T00:00:00Z".to_owned()),
            ..test_node(2, Some(ROOT_NODE_ID), "Draft", 0, "deleted body")
        };

        let ui_document = UiDocument::from_trash_nodes(&[deleted], UiLanguage::Korean)?;
        let draft = ui_document
            .node_by_id(2)
            .ok_or(DomainError::NodeNotFound { node_id: 2 })?;

        assert!(draft.title_sort_key.is_empty());
        Ok(())
    }

    #[test]
    fn trash_ui_document_sorts_unordered_nodes_for_display() -> Result<(), AppError> {
        let deleted_nodes = vec![
            deleted_test_node(4, Some(ROOT_NODE_ID), "Gamma", 2),
            deleted_test_node(ROOT_NODE_ID, None, "Root", 0),
            deleted_test_node(3, Some(ROOT_NODE_ID), "Beta", 1),
            deleted_test_node(2, Some(ROOT_NODE_ID), "Alpha", 0),
        ];

        let ui_document = UiDocument::from_trash_nodes(&deleted_nodes, UiLanguage::Korean)?;

        assert_eq!(ui_node_ids(&ui_document), vec![ROOT_NODE_ID, 2, 3, 4]);
        Ok(())
    }

    #[test]
    fn trash_ui_document_sorts_equal_order_nodes_by_title() -> Result<(), AppError> {
        let deleted_nodes = vec![
            deleted_test_node(ROOT_NODE_ID, None, "Root", 0),
            deleted_test_node(4, Some(ROOT_NODE_ID), "Gamma", 0),
            deleted_test_node(2, Some(ROOT_NODE_ID), "Alpha", 0),
            deleted_test_node(3, Some(ROOT_NODE_ID), "Beta", 0),
        ];

        let ui_document = UiDocument::from_trash_nodes(&deleted_nodes, UiLanguage::Korean)?;

        assert_eq!(ui_node_ids(&ui_document), vec![ROOT_NODE_ID, 2, 3, 4]);
        Ok(())
    }

    #[test]
    fn trash_tab_input_loads_deleted_body_on_demand() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let draft =
                repository.create_document_with_content(ROOT_NODE_ID, "Draft", "deleted body")?;
            let draft_id = draft.id;
            repository.soft_delete_node_cascade(draft_id)?;
            let app = App::from_repository_for_test(db_path.clone(), repository)?;
            let ui_settings = app.ui_settings();
            let deleted_nodes = app.deleted_nodes()?;
            let deleted_node = deleted_nodes
                .iter()
                .find(|node| node.id == draft_id)
                .ok_or(DomainError::NodeNotDeleted { node_id: draft_id })?;
            assert_eq!(deleted_node.content.as_str(), "");

            let document = UiDocument::from_trash_nodes(&deleted_nodes, ui_settings.language)?;
            let state = WindowState::new(app, document, ui_settings, DpiMetrics::system())?;

            let node_index = state
                .document
                .nodes
                .iter()
                .position(|node| node.id == draft_id)
                .ok_or(DomainError::NodeNotDeleted { node_id: draft_id })?;
            let trash_node = &state.document.nodes[node_index];
            assert_eq!(trash_node.title.as_str(), "Draft");

            let input = state
                .tab_input_from_ui_node_index(node_index)?
                .ok_or(DomainError::NodeNotDeleted { node_id: draft_id })?;
            assert_eq!(input.content.as_str(), "deleted body");
            assert_eq!(input.source, DocumentTabSource::Trash);
            assert!(!input.editable);
            Ok(())
        })();
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[test]
    fn trash_menu_state_enables_restore_and_permanent_delete_for_selection() {
        assert_eq!(
            MenuState::for_context(GuiTreeMode::Trash, false, None, true, true, false),
            MenuState {
                save_enabled: false,
                close_tab_enabled: false,
                new_child_document_enabled: false,
                new_document_enabled: false,
                rename_enabled: false,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: false,
                delete_permanently_enabled: false,
                active_tree_checked: false,
                trash_checked: true,
            }
        );

        assert_eq!(
            MenuState::for_context(GuiTreeMode::Trash, false, Some(42), true, true, true),
            MenuState {
                save_enabled: true,
                close_tab_enabled: true,
                new_child_document_enabled: false,
                new_document_enabled: false,
                rename_enabled: false,
                move_up_enabled: false,
                move_down_enabled: false,
                delete_enabled: false,
                restore_enabled: true,
                delete_permanently_enabled: true,
                active_tree_checked: false,
                trash_checked: true,
            }
        );
    }

    #[test]
    fn editor_menu_state_respects_editability_and_selection() {
        assert_eq!(
            EditorMenuState::for_context(true, true, true, true, true),
            EditorMenuState {
                undo_enabled: true,
                cut_enabled: true,
                copy_enabled: true,
                paste_enabled: true,
                delete_enabled: true,
                select_all_enabled: true,
                find_replace_enabled: true,
            }
        );

        assert_eq!(
            EditorMenuState::for_context(true, false, true, true, true),
            EditorMenuState {
                undo_enabled: false,
                cut_enabled: false,
                copy_enabled: true,
                paste_enabled: false,
                delete_enabled: false,
                select_all_enabled: true,
                find_replace_enabled: false,
            }
        );

        assert_eq!(
            EditorMenuState::for_context(false, false, true, true, true),
            EditorMenuState {
                undo_enabled: false,
                cut_enabled: false,
                copy_enabled: false,
                paste_enabled: false,
                delete_enabled: false,
                select_all_enabled: false,
                find_replace_enabled: false,
            }
        );
    }

    #[test]
    fn tree_label_edit_commits_notification_title() {
        let edit = TreeLabelEditState::new(7);

        assert_eq!(
            edit.commit_title(Some("Renamed".to_owned())),
            Some((7, "Renamed".to_owned()))
        );
    }

    #[test]
    fn tree_label_edit_commits_focus_loss_title_when_notification_has_no_text() {
        let mut edit = TreeLabelEditState::new(7);
        edit.remember_focus_loss_title("Focus Lost".to_owned());

        assert_eq!(edit.commit_title(None), Some((7, "Focus Lost".to_owned())));
    }

    #[test]
    fn tree_label_edit_cancel_rejects_focus_loss_title() {
        let mut edit = TreeLabelEditState::new(7);
        edit.remember_focus_loss_title("Focus Lost".to_owned());
        edit.mark_canceled();

        assert_eq!(edit.commit_title(None), None);
    }

    fn test_node(
        id: i64,
        parent_id: Option<i64>,
        title: &str,
        sort_order: i64,
        content: &str,
    ) -> Node {
        Node {
            id,
            parent_id,
            title: title.to_owned(),
            sort_order,
            content: content.to_owned(),
            created_at: "2026-04-30T00:00:00Z".to_owned(),
            updated_at: "2026-04-30T00:00:00Z".to_owned(),
            deleted_at: None,
        }
    }

    fn deleted_test_node(id: i64, parent_id: Option<i64>, title: &str, sort_order: i64) -> Node {
        Node {
            deleted_at: Some("2026-04-30T00:00:00Z".to_owned()),
            ..test_node(id, parent_id, title, sort_order, "")
        }
    }

    fn ui_node_ids(document: &UiDocument) -> Vec<i64> {
        document.nodes.iter().map(|node| node.id).collect()
    }

    fn unique_test_db_path() -> Result<PathBuf, Box<dyn Error>> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        Ok(std::env::temp_dir().join(format!(
            "j3treetext-state-{}-{nanos}-{counter}.db",
            std::process::id()
        )))
    }

    fn remove_file_if_exists(path: &Path) -> Result<(), Box<dyn Error>> {
        if path.exists() {
            fs::remove_file(path)?;
        }

        Ok(())
    }
}

pub(super) struct UiDocument {
    pub(super) nodes: Vec<UiNode>,
    node_ids: HashSet<i64>,
    node_indices_by_id: HashMap<i64, usize>,
    child_order: UiDocumentChildOrder,
}

impl UiDocument {
    pub(super) fn from_active_document(document: &Document) -> Result<Self, AppError> {
        Self::from_nodes(
            document.nodes(),
            false,
            UiLanguage::Korean,
            UiDocumentNodeOrder::TrustedDisplayOrder,
        )
    }

    pub(super) fn from_trash_nodes(nodes: &[Node], language: UiLanguage) -> Result<Self, AppError> {
        Self::from_nodes(nodes, true, language, UiDocumentNodeOrder::SortForDisplay)
    }

    pub(super) fn from_search_results(
        results: Vec<DocumentSearchResult>,
        language: UiLanguage,
    ) -> Result<Self, AppError> {
        let mut nodes = Vec::with_capacity(results.len());
        for result in results {
            nodes.push(UiNode::from_search_result(result, language)?);
        }
        Ok(Self::from_ui_nodes(nodes))
    }

    pub(super) fn from_ui_nodes(nodes: Vec<UiNode>) -> Self {
        Self::from_ui_nodes_with_child_order(nodes, UiDocumentChildOrder::SortForDisplay)
    }

    fn from_ui_nodes_with_child_order(
        nodes: Vec<UiNode>,
        child_order: UiDocumentChildOrder,
    ) -> Self {
        let (node_ids, node_indices_by_id) = ui_document_node_caches(&nodes);
        Self {
            nodes,
            node_ids,
            node_indices_by_id,
            child_order,
        }
    }

    fn from_nodes(
        source_nodes: &[Node],
        trash_mode: bool,
        language: UiLanguage,
        node_order: UiDocumentNodeOrder,
    ) -> Result<Self, AppError> {
        let deleted_ids: HashSet<i64> = if trash_mode {
            source_nodes.iter().map(|node| node.id).collect()
        } else {
            HashSet::new()
        };
        let mut nodes = Vec::with_capacity(source_nodes.len());

        for node in source_nodes {
            let display_parent_id = if trash_mode {
                node.parent_id
                    .filter(|parent_id| deleted_ids.contains(parent_id))
            } else {
                node.parent_id
            };
            nodes.push(UiNode::from_node(
                node,
                display_parent_id,
                trash_mode,
                language,
            )?);
        }

        if matches!(node_order, UiDocumentNodeOrder::SortForDisplay) {
            nodes.sort_by(compare_ui_nodes_for_display);
        }
        Ok(Self::from_ui_nodes_with_child_order(
            nodes,
            UiDocumentChildOrder::TrustedDisplayOrder,
        ))
    }

    pub(super) fn contains_node_id(&self, node_id: i64) -> bool {
        self.node_ids.contains(&node_id)
    }

    pub(super) fn node_by_id(&self, node_id: i64) -> Option<&UiNode> {
        self.node_index_by_id(node_id)
            .and_then(|index| self.nodes.get(index))
    }

    pub(super) fn node_index_by_id(&self, node_id: i64) -> Option<usize> {
        if let Some(index) = self.node_indices_by_id.get(&node_id).copied() {
            if self.nodes.get(index).is_some_and(|node| node.id == node_id) {
                return Some(index);
            }
        }

        self.nodes.iter().position(|node| node.id == node_id)
    }

    pub(super) fn child_indices_are_display_ordered(&self) -> bool {
        matches!(self.child_order, UiDocumentChildOrder::TrustedDisplayOrder)
    }

    pub(super) fn display_ordered_child_index_range(
        &self,
        parent_id: Option<i64>,
    ) -> Option<Range<usize>> {
        if !self.child_indices_are_display_ordered() {
            return None;
        }

        let start = self
            .nodes
            .partition_point(|node| node.display_parent_id < parent_id);
        let end =
            start + self.nodes[start..].partition_point(|node| node.display_parent_id == parent_id);
        Some(start..end)
    }

    pub(super) fn upsert_active_node_from_document(
        &mut self,
        document: &Document,
        node_id: i64,
    ) -> Result<usize, AppError> {
        let node = active_ui_node_from_document(document, node_id)?;
        if let Some(index) = self.node_index_by_id(node_id) {
            self.nodes.remove(index);
            let index = self.insert_node_preserving_child_order(node);
            self.rebuild_node_caches();
            return Ok(index);
        }

        let index = self.insert_node_preserving_child_order(node);
        self.node_ids.insert(node_id);
        self.rebuild_node_index_cache();
        Ok(index)
    }

    pub(super) fn remove_nodes_by_id(&mut self, node_ids: &[i64]) {
        if node_ids.is_empty() {
            return;
        }

        let mut removed_nodes = Vec::with_capacity(node_ids.len());
        for node_id in node_ids {
            if !self.node_ids.remove(node_id) {
                continue;
            }

            let cached_index = self.node_indices_by_id.remove(node_id).filter(|index| {
                self.nodes
                    .get(*index)
                    .is_some_and(|node| node.id == *node_id)
            });
            let index =
                cached_index.or_else(|| self.nodes.iter().position(|node| node.id == *node_id));
            if let Some(index) = index {
                removed_nodes.push((index, *node_id));
            }
        }
        if removed_nodes.is_empty() {
            return;
        }

        let mut first_changed_index = self.nodes.len();
        let mut indexed_shift_cost = 0usize;
        for (index, _) in &removed_nodes {
            first_changed_index = first_changed_index.min(*index);
            indexed_shift_cost =
                indexed_shift_cost.saturating_add(self.nodes.len().saturating_sub(*index + 1));
        }

        if indexed_shift_cost <= self.nodes.len() {
            removed_nodes.sort_unstable_by_key(|removed| std::cmp::Reverse(removed.0));
            for (index, node_id) in removed_nodes {
                if self.nodes.get(index).is_some_and(|node| node.id == node_id) {
                    self.nodes.remove(index);
                } else if let Some(index) = self.nodes.iter().position(|node| node.id == node_id) {
                    first_changed_index = first_changed_index.min(index);
                    self.nodes.remove(index);
                }
            }
        } else {
            let removed_ids: HashSet<i64> =
                removed_nodes.iter().map(|(_, node_id)| *node_id).collect();
            self.nodes.retain(|node| !removed_ids.contains(&node.id));
        }

        self.refresh_node_index_cache_for_range(first_changed_index..self.nodes.len());
    }

    pub(super) fn sync_active_nodes_after_move(
        &mut self,
        document: &Document,
        moved_node_id: i64,
        old_parent_id: Option<i64>,
        _new_parent_id: Option<i64>,
    ) -> Result<(), AppError> {
        if self.child_indices_are_display_ordered() {
            return self.sync_display_ordered_active_nodes_after_move(
                document,
                moved_node_id,
                old_parent_id,
            );
        }

        self.sync_active_nodes_after_move_by_scan(document, moved_node_id, old_parent_id)
    }

    fn sync_active_nodes_after_move_by_scan(
        &mut self,
        document: &Document,
        moved_node_id: i64,
        old_parent_id: Option<i64>,
    ) -> Result<(), AppError> {
        let moved_node = active_ui_node_from_document(document, moved_node_id)?;
        let new_parent_id = moved_node.display_parent_id;
        for index in 0..self.nodes.len() {
            let node_id = self.nodes[index].id;
            let display_parent_id = self.nodes[index].display_parent_id;
            let affected = node_id == moved_node_id
                || display_parent_id == old_parent_id
                || display_parent_id == new_parent_id;
            if affected {
                self.nodes[index] = active_ui_node_from_document(document, node_id)?;
            }
        }
        if self.child_indices_are_display_ordered() {
            self.nodes.sort_by(compare_ui_nodes_for_display);
            self.rebuild_node_index_cache();
        }

        Ok(())
    }

    fn sync_display_ordered_active_nodes_after_move(
        &mut self,
        document: &Document,
        moved_node_id: i64,
        old_parent_id: Option<i64>,
    ) -> Result<(), AppError> {
        let moved_node = active_ui_node_from_document(document, moved_node_id)?;
        let new_parent_id = moved_node.display_parent_id;
        let Some(old_range) = self.display_ordered_child_index_range(old_parent_id) else {
            return self.sync_active_nodes_after_move_by_scan(
                document,
                moved_node_id,
                old_parent_id,
            );
        };
        if !old_range
            .clone()
            .any(|index| self.nodes[index].id == moved_node_id)
        {
            return self.sync_active_nodes_after_move_by_scan(
                document,
                moved_node_id,
                old_parent_id,
            );
        }

        if old_parent_id == new_parent_id {
            self.refresh_display_ordered_child_range(document, old_range)?;
            return Ok(());
        }

        let mut old_nodes =
            self.active_ui_nodes_for_range(document, old_range.clone(), Some(moved_node_id))?;
        old_nodes.sort_by(compare_ui_nodes_for_display);
        let _ = self.nodes.splice(old_range, old_nodes);

        let Some(new_range) = self.display_ordered_child_index_range(new_parent_id) else {
            return self.sync_active_nodes_after_move_by_scan(
                document,
                moved_node_id,
                old_parent_id,
            );
        };
        let mut new_nodes = self.active_ui_nodes_for_range(document, new_range.clone(), None)?;
        new_nodes.push(moved_node);
        new_nodes.sort_by(compare_ui_nodes_for_display);
        let _ = self.nodes.splice(new_range, new_nodes);

        let Some(old_range) = self.display_ordered_child_index_range(old_parent_id) else {
            return self.sync_active_nodes_after_move_by_scan(
                document,
                moved_node_id,
                old_parent_id,
            );
        };
        let Some(new_range) = self.display_ordered_child_index_range(new_parent_id) else {
            return self.sync_active_nodes_after_move_by_scan(
                document,
                moved_node_id,
                old_parent_id,
            );
        };
        self.refresh_node_index_cache_for_range(
            old_range.start.min(new_range.start)..old_range.end.max(new_range.end),
        );

        Ok(())
    }

    fn refresh_display_ordered_child_range(
        &mut self,
        document: &Document,
        range: Range<usize>,
    ) -> Result<(), AppError> {
        let start = range.start;
        let mut nodes = self.active_ui_nodes_for_range(document, range, None)?;
        nodes.sort_by(compare_ui_nodes_for_display);
        let end = start + nodes.len();
        let _ = self.nodes.splice(start..end, nodes);
        self.refresh_node_index_cache_for_range(start..end);
        Ok(())
    }

    fn active_ui_nodes_for_range(
        &self,
        document: &Document,
        range: Range<usize>,
        excluded_node_id: Option<i64>,
    ) -> Result<Vec<UiNode>, AppError> {
        let mut nodes = Vec::with_capacity(range.len());
        for index in range {
            let node_id = self.nodes[index].id;
            if Some(node_id) == excluded_node_id {
                continue;
            }
            nodes.push(active_ui_node_from_document(document, node_id)?);
        }
        Ok(nodes)
    }

    fn refresh_node_index_cache_for_range(&mut self, range: Range<usize>) {
        let end = range.end.min(self.nodes.len());
        for index in range.start..end {
            if let Some(node) = self.nodes.get(index) {
                self.node_indices_by_id.insert(node.id, index);
            }
        }
    }

    fn insert_node_preserving_child_order(&mut self, node: UiNode) -> usize {
        if self.child_indices_are_display_ordered() {
            let index = match self
                .nodes
                .binary_search_by(|existing| compare_ui_nodes_for_display(existing, &node))
            {
                Ok(index) | Err(index) => index,
            };
            self.nodes.insert(index, node);
            index
        } else {
            self.nodes.push(node);
            self.nodes.len() - 1
        }
    }

    fn rebuild_node_caches(&mut self) {
        let (node_ids, node_indices_by_id) = ui_document_node_caches(&self.nodes);
        self.node_ids = node_ids;
        self.node_indices_by_id = node_indices_by_id;
    }

    fn rebuild_node_index_cache(&mut self) {
        self.node_indices_by_id = self
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id, index))
            .collect();
    }
}

enum UiDocumentNodeOrder {
    TrustedDisplayOrder,
    SortForDisplay,
}

#[derive(Clone, Copy)]
enum UiDocumentChildOrder {
    TrustedDisplayOrder,
    SortForDisplay,
}

fn ui_document_node_caches(nodes: &[UiNode]) -> (HashSet<i64>, HashMap<i64, usize>) {
    let mut node_ids = HashSet::with_capacity(nodes.len());
    let mut node_indices_by_id = HashMap::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        node_ids.insert(node.id);
        node_indices_by_id.insert(node.id, index);
    }
    (node_ids, node_indices_by_id)
}

fn active_ui_node_from_document(document: &Document, node_id: i64) -> Result<UiNode, AppError> {
    let node = document
        .node_by_id(node_id)
        .ok_or(DomainError::NodeNotFound { node_id })?;
    UiNode::from_node(node, node.parent_id, false, UiLanguage::Korean)
}

pub(super) fn compare_ui_nodes_for_display(left: &UiNode, right: &UiNode) -> Ordering {
    (
        left.display_parent_id,
        left.sort_order,
        left.title.as_str(),
        left.id,
    )
        .cmp(&(
            right.display_parent_id,
            right.sort_order,
            right.title.as_str(),
            right.id,
        ))
}

pub(super) struct UiNode {
    pub(super) id: i64,
    pub(super) parent_id: Option<i64>,
    pub(super) display_parent_id: Option<i64>,
    pub(super) title: String,
    pub(super) sort_order: i64,
    pub(super) title_sort_key: String,
    pub(super) display_title: Vec<u16>,
    pub(super) search_content_matched: bool,
    pub(super) updated_at: String,
    pub(super) editable: bool,
    pub(super) source: DocumentTabSource,
}

impl UiNode {
    fn from_node(
        node: &Node,
        display_parent_id: Option<i64>,
        trash_mode: bool,
        language: UiLanguage,
    ) -> Result<Self, AppError> {
        let display_title = if trash_mode {
            Cow::Owned(ui_text(language).deleted_title(&node.title))
        } else {
            Cow::Borrowed(node.title.as_str())
        };
        let display_title = utf8_to_wide_null(
            "convert node title from UTF-8 to UTF-16",
            display_title.as_ref(),
        )?;
        Ok(Self {
            id: node.id,
            parent_id: node.parent_id,
            display_parent_id,
            title: node.title.clone(),
            sort_order: node.sort_order,
            title_sort_key: String::new(),
            display_title,
            search_content_matched: false,
            updated_at: node.updated_at.clone(),
            editable: !trash_mode,
            source: if trash_mode {
                DocumentTabSource::Trash
            } else {
                DocumentTabSource::ActiveTree
            },
        })
    }

    fn from_search_result(
        result: DocumentSearchResult,
        language: UiLanguage,
    ) -> Result<Self, AppError> {
        let DocumentSearchResult {
            node,
            parent_title,
            content_matched,
        } = result;
        let display_title =
            ui_text(language).search_result_title(&node.title, parent_title.as_deref());
        let display_title = utf8_to_wide_null(
            "convert search result title from UTF-8 to UTF-16",
            &display_title,
        )?;
        Ok(Self {
            id: node.id,
            parent_id: node.parent_id,
            display_parent_id: None,
            title: node.title,
            sort_order: node.sort_order,
            title_sort_key: String::new(),
            display_title,
            search_content_matched: content_matched,
            updated_at: node.updated_at,
            editable: true,
            source: DocumentTabSource::SearchResult,
        })
    }
}

fn tab_input_from_ui_node(
    node: &UiNode,
    content: String,
    loaded_updated_at: String,
) -> OpenDocumentTabInput {
    OpenDocumentTabInput {
        node_id: node.id,
        parent_id: node.parent_id,
        title: node.title.clone(),
        content,
        loaded_updated_at,
        editable: node.editable,
        source: node.source,
    }
}

#[derive(Clone, Copy)]
pub(super) struct InitialSelection {
    pub(super) node_index: usize,
    pub(super) handle: HTREEITEM,
}

pub(super) struct TreePopulation {
    preferred_node_id: Option<i64>,
    preferred: Option<InitialSelection>,
    first_document: Option<InitialSelection>,
    first_item: Option<InitialSelection>,
    item_handles_by_node_id: TreeItemHandlesByNodeId,
}

impl TreePopulation {
    pub(super) fn new(preferred_node_id: Option<i64>, expected_items: usize) -> Self {
        Self {
            preferred_node_id,
            preferred: None,
            first_document: None,
            first_item: None,
            item_handles_by_node_id: TreeItemHandlesByNodeId::with_capacity(expected_items),
        }
    }

    pub(super) fn remember(&mut self, node: &UiNode, node_index: usize, handle: HTREEITEM) {
        let selection = InitialSelection { node_index, handle };
        self.item_handles_by_node_id.insert(node.id, handle);
        if self.preferred_node_id == Some(node.id) {
            self.preferred = Some(selection);
        }
        if self.first_document.is_none() {
            self.first_document = Some(selection);
        }
        if self.first_item.is_none() {
            self.first_item = Some(selection);
        }
    }

    pub(super) fn into_parts(self) -> (Option<InitialSelection>, TreeItemHandlesByNodeId) {
        (
            self.preferred.or(self.first_document).or(self.first_item),
            self.item_handles_by_node_id,
        )
    }
}
