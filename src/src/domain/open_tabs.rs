use std::collections::HashSet;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentTabSource {
    ActiveTree,
    SearchResult,
    Trash,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenDocumentTabInput {
    pub node_id: i64,
    pub parent_id: Option<i64>,
    pub title: String,
    pub content: String,
    pub loaded_updated_at: String,
    pub editable: bool,
    pub source: DocumentTabSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedTabMetadataUpdate<'a> {
    pub node_id: i64,
    pub parent_id: Option<i64>,
    pub title: String,
    pub loaded_updated_at: String,
    pub editable: bool,
    pub source: DocumentTabSource,
    pub current_content_for_dirty_token: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DocumentTabViewState {
    pub first_visible_line: usize,
    pub caret_position_utf16: usize,
    pub selection_start_utf16: usize,
    pub selection_end_utf16: usize,
}

impl DocumentTabViewState {
    pub fn clamped(self, max_text_offset_utf16: usize, max_first_visible_line: usize) -> Self {
        let caret_position_utf16 = self.caret_position_utf16.min(max_text_offset_utf16);
        if self.selection_start_utf16 == self.selection_end_utf16 {
            return Self {
                first_visible_line: self.first_visible_line.min(max_first_visible_line),
                caret_position_utf16,
                selection_start_utf16: caret_position_utf16,
                selection_end_utf16: caret_position_utf16,
            };
        }

        let selection_start = self.selection_start_utf16.min(max_text_offset_utf16);
        let selection_end = self.selection_end_utf16.min(max_text_offset_utf16);
        let (selection_start_utf16, selection_end_utf16) = if selection_start <= selection_end {
            (selection_start, selection_end)
        } else {
            (selection_end, selection_start)
        };
        let caret_position_utf16 =
            caret_position_utf16.clamp(selection_start_utf16, selection_end_utf16);

        Self {
            first_visible_line: self.first_visible_line.min(max_first_visible_line),
            caret_position_utf16,
            selection_start_utf16,
            selection_end_utf16,
        }
    }
}

#[derive(Debug, Clone, Eq)]
pub struct OpenDocumentTab {
    pub node_id: i64,
    pub parent_id: Option<i64>,
    pub title: String,
    pub content: String,
    loaded_content: LoadedContent,
    pub loaded_updated_at: String,
    pub dirty: bool,
    pub editable: bool,
    pub source: DocumentTabSource,
    pub view_state: DocumentTabViewState,
    content_revision: u64,
}

impl PartialEq for OpenDocumentTab {
    fn eq(&self, other: &Self) -> bool {
        self.node_id == other.node_id
            && self.parent_id == other.parent_id
            && self.title == other.title
            && self.content == other.content
            && self.loaded_content == other.loaded_content
            && self.loaded_updated_at == other.loaded_updated_at
            && self.dirty == other.dirty
            && self.editable == other.editable
            && self.source == other.source
            && self.view_state == other.view_state
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LoadedContent {
    Current,
    Saved(String),
}

impl LoadedContent {
    fn as_str<'a>(&'a self, current_content: &'a str) -> &'a str {
        match self {
            Self::Current => current_content,
            Self::Saved(content) => content.as_str(),
        }
    }
}

impl OpenDocumentTab {
    pub fn from_input(input: OpenDocumentTabInput) -> Self {
        Self {
            node_id: input.node_id,
            parent_id: input.parent_id,
            title: input.title,
            content: input.content,
            loaded_content: LoadedContent::Current,
            loaded_updated_at: input.loaded_updated_at,
            dirty: false,
            editable: input.editable,
            source: input.source,
            view_state: DocumentTabViewState::default(),
            content_revision: 0,
        }
    }

    pub fn loaded_content(&self) -> &str {
        self.loaded_content.as_str(&self.content)
    }

    pub fn content_revision(&self) -> u64 {
        self.content_revision
    }

    pub fn display_title(&self) -> String {
        self.title.clone()
    }

    pub fn is_save_target(&self) -> bool {
        self.editable && self.dirty
    }

    pub fn has_unsavable_changes(&self) -> bool {
        self.dirty && !self.is_save_target()
    }

    pub fn set_content(&mut self, content: String) -> bool {
        if !self.editable {
            return false;
        }

        let previous_dirty = self.dirty;
        let content_changed = self.content != content;

        match &self.loaded_content {
            LoadedContent::Current if content_changed => {
                let loaded_content = std::mem::replace(&mut self.content, content);
                self.loaded_content = LoadedContent::Saved(loaded_content);
                self.dirty = true;
                self.bump_content_revision();
                content_changed || previous_dirty != self.dirty
            }
            LoadedContent::Current => {
                self.content = content;
                self.dirty = false;
                content_changed || previous_dirty != self.dirty
            }
            LoadedContent::Saved(loaded_content) => {
                self.content = content;
                let next_dirty = self.content != loaded_content.as_str();
                if !next_dirty {
                    self.loaded_content = LoadedContent::Current;
                }
                self.dirty = next_dirty;
                if content_changed {
                    self.bump_content_revision();
                }
                content_changed || previous_dirty != self.dirty
            }
        }
    }

    pub fn set_content_reusing(&mut self, content: &mut String) -> bool {
        if !self.editable {
            return false;
        }

        let previous_dirty = self.dirty;
        let content_changed = self.content.as_str() != content.as_str();
        std::mem::swap(&mut self.content, content);

        let next_dirty = match &self.loaded_content {
            LoadedContent::Current if content_changed => {
                self.loaded_content = LoadedContent::Saved(std::mem::take(content));
                true
            }
            LoadedContent::Current => false,
            LoadedContent::Saved(loaded_content) => self.content != loaded_content.as_str(),
        };

        if !next_dirty {
            self.loaded_content = LoadedContent::Current;
        }
        self.dirty = next_dirty;
        if content_changed {
            self.bump_content_revision();
        }
        content_changed || previous_dirty != self.dirty
    }

    fn replace_content_range(&mut self, range: Range<usize>, replacement: &str) -> bool {
        if !self.editable
            || range.start > range.end
            || range.end > self.content.len()
            || !self.content.is_char_boundary(range.start)
            || !self.content.is_char_boundary(range.end)
        {
            return false;
        }
        if self.content.get(range.clone()) == Some(replacement) {
            return false;
        }

        if matches!(self.loaded_content, LoadedContent::Current) {
            self.loaded_content = LoadedContent::Saved(self.content.clone());
        }

        self.content.replace_range(range, replacement);
        let next_dirty = match &self.loaded_content {
            LoadedContent::Current => false,
            LoadedContent::Saved(loaded_content) => self.content != loaded_content.as_str(),
        };
        if !next_dirty {
            self.loaded_content = LoadedContent::Current;
        }
        self.dirty = next_dirty;
        self.bump_content_revision();
        true
    }

    pub fn import_content(&mut self, content: String) -> bool {
        if !self.editable {
            return false;
        }

        let content_changed = self.content != content;
        let changed = content_changed || !self.dirty;
        match &self.loaded_content {
            LoadedContent::Current if self.content != content => {
                let loaded_content = std::mem::replace(&mut self.content, content);
                self.loaded_content = LoadedContent::Saved(loaded_content);
            }
            LoadedContent::Current => {
                self.loaded_content = LoadedContent::Saved(content);
            }
            _ => {
                self.content = content;
            }
        }
        if content_changed {
            self.bump_content_revision();
        }
        self.dirty = true;
        self.view_state = DocumentTabViewState::default();
        changed
    }

    pub fn mark_dirty_from_view(&mut self) -> bool {
        if !self.editable || self.dirty {
            return false;
        }

        self.dirty = true;
        true
    }

    pub fn mark_saved(&mut self, content: String, updated_at: String) {
        let content_changed = self.content != content;
        self.content = content;
        self.loaded_content = LoadedContent::Current;
        self.loaded_updated_at = updated_at;
        self.dirty = false;
        self.editable = true;
        self.source = DocumentTabSource::ActiveTree;
        if content_changed {
            self.bump_content_revision();
        }
    }

    pub fn mark_current_content_saved(&mut self, updated_at: String) {
        self.loaded_content = LoadedContent::Current;
        self.loaded_updated_at = updated_at;
        self.dirty = false;
        self.editable = true;
        self.source = DocumentTabSource::ActiveTree;
    }

    pub fn reload_from(&mut self, input: OpenDocumentTabInput) {
        let content_changed = self.content != input.content;
        self.node_id = input.node_id;
        self.parent_id = input.parent_id;
        self.title = input.title;
        self.content = input.content;
        self.loaded_content = LoadedContent::Current;
        self.loaded_updated_at = input.loaded_updated_at;
        self.dirty = false;
        self.editable = input.editable;
        self.source = input.source;
        self.view_state = DocumentTabViewState::default();
        if content_changed {
            self.bump_content_revision();
        }
    }

    pub fn sync_from(&mut self, input: OpenDocumentTabInput, update_dirty_token: bool) {
        self.parent_id = input.parent_id;
        self.title = input.title;
        self.editable = input.editable;
        self.source = input.source;

        if !self.dirty {
            let content_changed = self.content != input.content;
            self.content = input.content;
            self.loaded_content = LoadedContent::Current;
            self.loaded_updated_at = input.loaded_updated_at;
            if content_changed {
                self.bump_content_revision();
            }
        } else if update_dirty_token && self.loaded_content() == input.content {
            self.loaded_updated_at = input.loaded_updated_at;
        }
    }

    fn sync_metadata_preserving_content(
        &mut self,
        parent_id: Option<i64>,
        title: String,
        loaded_updated_at: String,
        editable: bool,
        source: DocumentTabSource,
        current_content_for_dirty_token: Option<&str>,
    ) {
        self.parent_id = parent_id;
        self.title = title;
        self.editable = editable;
        self.source = source;

        if !self.dirty {
            self.loaded_content = LoadedContent::Current;
            self.loaded_updated_at = loaded_updated_at;
        } else if let Some(current_content) = current_content_for_dirty_token {
            if self.loaded_content() == current_content {
                self.loaded_updated_at = loaded_updated_at;
            }
        }
    }

    fn mark_read_only(&mut self) -> bool {
        if !self.editable {
            return false;
        }

        self.editable = false;
        true
    }

    pub fn discard_changes(&mut self) {
        if let LoadedContent::Saved(loaded_content) =
            std::mem::replace(&mut self.loaded_content, LoadedContent::Current)
        {
            let content_changed = self.content != loaded_content;
            self.content = loaded_content;
            if content_changed {
                self.bump_content_revision();
            }
        }
        self.dirty = false;
        self.view_state = DocumentTabViewState::default();
    }

    pub fn set_view_state(&mut self, view_state: DocumentTabViewState) {
        self.view_state = view_state;
    }

    fn bump_content_revision(&mut self) {
        self.content_revision = self.content_revision.saturating_add(1);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenTabResult {
    Opened { index: usize },
    ActivatedExisting { index: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpenTabs {
    tabs: Vec<OpenDocumentTab>,
    active_index: Option<usize>,
}

impl OpenTabs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tabs(&self) -> &[OpenDocumentTab] {
        &self.tabs
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active_index
    }

    pub fn active(&self) -> Option<&OpenDocumentTab> {
        self.active_index.and_then(|index| self.tabs.get(index))
    }

    pub fn active_mut(&mut self) -> Option<&mut OpenDocumentTab> {
        self.active_index.and_then(|index| self.tabs.get_mut(index))
    }

    pub fn has_active(&self) -> bool {
        self.active_index.is_some()
    }

    pub fn first_dirty_index(&self) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.dirty)
    }

    pub fn dirty_tab_indices_for_nodes(&self, node_ids: &[i64]) -> Vec<usize> {
        let node_ids = node_id_set(node_ids);
        self.dirty_tab_indices_for_node_set(&node_ids)
    }

    pub fn dirty_tab_indices_for_node_set(&self, node_ids: &HashSet<i64>) -> Vec<usize> {
        self.tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                (tab.dirty && node_ids.contains(&tab.node_id)).then_some(index)
            })
            .collect()
    }

    pub fn open_or_activate(&mut self, input: OpenDocumentTabInput) -> OpenTabResult {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.node_id == input.node_id)
        {
            self.tabs[index].sync_from(input, false);
            self.active_index = Some(index);
            return OpenTabResult::ActivatedExisting { index };
        }

        let index = self.tabs.len();
        self.tabs.push(OpenDocumentTab::from_input(input));
        self.active_index = Some(index);
        OpenTabResult::Opened { index }
    }

    pub fn set_active(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }

        self.active_index = Some(index);
        true
    }

    pub fn set_active_by_node_id(&mut self, node_id: i64) -> bool {
        match self.tabs.iter().position(|tab| tab.node_id == node_id) {
            Some(index) => self.set_active(index),
            None => false,
        }
    }

    pub fn move_tab(&mut self, from_index: usize, to_index: usize) -> bool {
        if from_index >= self.tabs.len() || to_index >= self.tabs.len() || from_index == to_index {
            return false;
        }

        let moved = self.tabs.remove(from_index);
        self.tabs.insert(to_index, moved);

        self.active_index = self.active_index.map(|active| {
            if active == from_index {
                to_index
            } else if from_index < active && active <= to_index {
                active - 1
            } else if to_index <= active && active < from_index {
                active + 1
            } else {
                active
            }
        });
        true
    }

    pub fn update_active_content(&mut self, content: String) -> bool {
        match self.active_mut() {
            Some(tab) => tab.set_content(content),
            None => false,
        }
    }

    pub fn update_active_content_reusing(&mut self, content: &mut String) -> bool {
        match self.active_mut() {
            Some(tab) => tab.set_content_reusing(content),
            None => false,
        }
    }

    pub fn replace_active_content_range(&mut self, range: Range<usize>, replacement: &str) -> bool {
        match self.active_mut() {
            Some(tab) => tab.replace_content_range(range, replacement),
            None => false,
        }
    }

    pub fn import_active_content(&mut self, content: String) -> bool {
        match self.active_mut() {
            Some(tab) => tab.import_content(content),
            None => false,
        }
    }

    pub fn mark_active_dirty_from_view(&mut self) -> bool {
        match self.active_mut() {
            Some(tab) => tab.mark_dirty_from_view(),
            None => false,
        }
    }

    pub fn update_active_view_state(&mut self, view_state: DocumentTabViewState) -> bool {
        match self.active_mut() {
            Some(tab) => {
                tab.set_view_state(view_state);
                true
            }
            None => false,
        }
    }

    pub fn mark_active_saved(&mut self, content: String, updated_at: String) -> bool {
        match self.active_mut() {
            Some(tab) => {
                tab.mark_saved(content, updated_at);
                true
            }
            None => false,
        }
    }

    pub fn mark_active_current_content_saved(&mut self, updated_at: String) -> bool {
        match self.active_mut() {
            Some(tab) => {
                tab.mark_current_content_saved(updated_at);
                true
            }
            None => false,
        }
    }

    pub fn reload_active(&mut self, input: OpenDocumentTabInput) -> bool {
        match self.active_mut() {
            Some(tab) => {
                tab.reload_from(input);
                true
            }
            None => false,
        }
    }

    pub fn replace_active(&mut self, input: OpenDocumentTabInput) -> bool {
        let Some(mut active_index) = self.active_index else {
            return false;
        };

        if let Some(duplicate_index) = self.tabs.iter().enumerate().find_map(|(index, tab)| {
            (index != active_index && tab.node_id == input.node_id).then_some(index)
        }) {
            self.tabs.remove(duplicate_index);
            if duplicate_index < active_index {
                active_index -= 1;
                self.active_index = Some(active_index);
            }
        }

        if let Some(tab) = self.tabs.get_mut(active_index) {
            let previous_content_revision = tab.content_revision();
            *tab = OpenDocumentTab::from_input(input);
            tab.content_revision = previous_content_revision.saturating_add(1);
            true
        } else {
            false
        }
    }

    pub fn sync_loaded_tab(&mut self, input: OpenDocumentTabInput, update_dirty_token: bool) {
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.node_id == input.node_id)
        {
            self.tabs[index].sync_from(input, update_dirty_token);
        }
    }

    pub fn sync_loaded_tab_metadata(
        &mut self,
        node_id: i64,
        parent_id: Option<i64>,
        title: String,
        editable: bool,
        source: DocumentTabSource,
    ) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.node_id == node_id) {
            self.sync_loaded_tab_metadata_at(index, node_id, parent_id, title, editable, source);
        }
    }

    pub fn sync_loaded_tab_metadata_at(
        &mut self,
        index: usize,
        node_id: i64,
        parent_id: Option<i64>,
        title: String,
        editable: bool,
        source: DocumentTabSource,
    ) -> bool {
        let Some(tab) = self.tabs.get_mut(index) else {
            return false;
        };
        if tab.node_id != node_id {
            return false;
        }

        tab.parent_id = parent_id;
        tab.title = title;
        tab.editable = editable;
        tab.source = source;
        true
    }

    pub fn sync_loaded_tab_metadata_preserving_content(
        &mut self,
        node_id: i64,
        parent_id: Option<i64>,
        title: String,
        loaded_updated_at: String,
        editable: bool,
        source: DocumentTabSource,
    ) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.node_id == node_id) {
            self.tabs[index].sync_metadata_preserving_content(
                parent_id,
                title,
                loaded_updated_at,
                editable,
                source,
                None,
            );
        }
    }

    pub fn sync_loaded_tab_metadata_preserving_content_at(
        &mut self,
        index: usize,
        update: LoadedTabMetadataUpdate<'_>,
    ) -> bool {
        let Some(tab) = self.tabs.get_mut(index) else {
            return false;
        };
        if tab.node_id != update.node_id {
            return false;
        }

        tab.sync_metadata_preserving_content(
            update.parent_id,
            update.title,
            update.loaded_updated_at,
            update.editable,
            update.source,
            update.current_content_for_dirty_token,
        );
        true
    }

    pub fn mark_tabs_missing_from_active_document_read_only(
        &mut self,
        active_node_ids: &[i64],
    ) -> bool {
        let active_node_id = self.active().map(|tab| tab.node_id);
        let mut active_marked_read_only = false;

        for tab in &mut self.tabs {
            if tab.editable
                && !active_node_ids.contains(&tab.node_id)
                && tab.mark_read_only()
                && active_node_id == Some(tab.node_id)
            {
                active_marked_read_only = true;
            }
        }

        active_marked_read_only
    }

    pub fn discard_tab_changes(&mut self, index: usize) -> bool {
        match self.tabs.get_mut(index) {
            Some(tab) => {
                tab.discard_changes();
                true
            }
            None => false,
        }
    }

    pub fn close_active(&mut self) -> Option<OpenDocumentTab> {
        let index = self.active_index?;
        self.close_at(index)
    }

    pub fn close_tabs_for_nodes(&mut self, node_ids: &[i64]) -> Vec<OpenDocumentTab> {
        let node_ids = node_id_set(node_ids);
        self.close_tabs_for_node_set(&node_ids)
    }

    pub fn close_tabs_for_node_set(&mut self, node_ids: &HashSet<i64>) -> Vec<OpenDocumentTab> {
        let mut closed = Vec::new();
        let mut index = 0;
        while index < self.tabs.len() {
            if node_ids.contains(&self.tabs[index].node_id) {
                if let Some(tab) = self.close_at(index) {
                    closed.push(tab);
                }
            } else {
                index += 1;
            }
        }
        closed
    }

    fn close_at(&mut self, index: usize) -> Option<OpenDocumentTab> {
        if index >= self.tabs.len() {
            return None;
        }

        let previous_active = self.active_index;
        let removed = self.tabs.remove(index);
        self.active_index = match (self.tabs.is_empty(), previous_active) {
            (true, _) => None,
            (false, Some(active)) if active == index => Some(index.min(self.tabs.len() - 1)),
            (false, Some(active)) if active > index => Some(active - 1),
            (false, Some(active)) => Some(active),
            (false, None) => None,
        };
        Some(removed)
    }
}

fn node_id_set(node_ids: &[i64]) -> HashSet<i64> {
    node_ids.iter().copied().collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyTabDecision {
    Save,
    Discard,
    Cancel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_tabs_use_current_content_as_loaded_baseline() {
        let mut tab = OpenDocumentTab::from_input(input("loaded", "v1"));
        assert_loaded_is_current(&tab);
        assert_eq!(tab.loaded_content(), "loaded");
        assert!(!tab.dirty);

        tab.mark_saved("saved".to_owned(), "v2".to_owned());
        assert_loaded_is_current(&tab);
        assert_eq!(tab.content, "saved");
        assert_eq!(tab.loaded_updated_at, "v2");
        assert!(!tab.dirty);

        assert!(tab.set_content("draft".to_owned()));
        tab.mark_current_content_saved("v3".to_owned());
        assert_loaded_is_current(&tab);
        assert_eq!(tab.loaded_content(), "draft");
        assert_eq!(tab.loaded_updated_at, "v3");
        assert!(!tab.dirty);

        tab.reload_from(input("reloaded", "v4"));
        assert_loaded_is_current(&tab);
        assert_eq!(tab.content, "reloaded");
        assert_eq!(tab.loaded_updated_at, "v4");
        assert!(!tab.dirty);

        tab.sync_from(input("synced", "v5"), false);
        assert_loaded_is_current(&tab);
        assert_eq!(tab.content, "synced");
        assert_eq!(tab.loaded_updated_at, "v5");
        assert!(!tab.dirty);
    }

    #[test]
    fn set_content_materializes_and_releases_saved_baseline() {
        let mut tab = OpenDocumentTab::from_input(input("saved", "v1"));

        assert!(tab.set_content("draft".to_owned()));
        assert_eq!(tab.content, "draft");
        assert_loaded_is_saved(&tab, "saved");
        assert!(tab.dirty);

        assert!(tab.set_content("saved".to_owned()));
        assert_eq!(tab.content, "saved");
        assert_loaded_is_current(&tab);
        assert!(!tab.dirty);
    }

    #[test]
    fn content_revision_changes_only_with_visible_content() {
        let mut tab = OpenDocumentTab::from_input(input("saved", "v1"));
        let initial_revision = tab.content_revision();

        assert!(!tab.set_content("saved".to_owned()));
        assert_eq!(tab.content_revision(), initial_revision);

        assert!(tab.mark_dirty_from_view());
        assert_eq!(tab.content_revision(), initial_revision);

        assert!(tab.set_content("draft".to_owned()));
        let draft_revision = tab.content_revision();
        assert!(draft_revision > initial_revision);

        tab.mark_current_content_saved("v2".to_owned());
        assert_eq!(tab.content_revision(), draft_revision);

        tab.mark_saved("stored".to_owned(), "v3".to_owned());
        assert!(tab.content_revision() > draft_revision);
    }

    #[test]
    fn set_content_reusing_moves_previous_buffer_into_saved_baseline() {
        let mut tab = OpenDocumentTab::from_input(input("saved", "v1"));
        let saved_content_ptr = tab.content.as_ptr();
        let mut next_content = String::from("draft");

        assert!(tab.set_content_reusing(&mut next_content));
        assert_eq!(tab.content, "draft");
        assert!(next_content.is_empty());
        assert_loaded_is_saved(&tab, "saved");
        if let LoadedContent::Saved(content) = &tab.loaded_content {
            assert_eq!(content.as_ptr(), saved_content_ptr);
        }
        assert!(tab.dirty);

        let mut saved_content = String::from("saved");
        assert!(tab.set_content_reusing(&mut saved_content));
        assert_eq!(tab.content, "saved");
        assert_eq!(saved_content, "draft");
        assert_loaded_is_current(&tab);
        assert!(!tab.dirty);
    }

    #[test]
    fn replace_active_content_range_preserves_saved_baseline() {
        let mut tabs = OpenTabs::new();
        tabs.open_or_activate(input("one two one", "v1"));

        assert!(tabs.replace_active_content_range(4..7, "dos"));
        let tab = tabs.active().expect("active tab should exist");
        assert_eq!(tab.content, "one dos one");
        assert_loaded_is_saved(tab, "one two one");
        assert!(tab.dirty);
        let content_ptr = tab.content.as_ptr();
        let revision = tab.content_revision();

        assert!(tabs.replace_active_content_range(4..7, "two"));
        let tab = tabs.active().expect("active tab should exist");
        assert_eq!(tab.content, "one two one");
        assert_eq!(tab.content.as_ptr(), content_ptr);
        assert_loaded_is_current(tab);
        assert!(!tab.dirty);
        assert!(tab.content_revision() > revision);
    }

    #[test]
    fn discard_changes_restores_saved_content_and_drops_baseline() {
        let mut tab = OpenDocumentTab::from_input(input("saved", "v1"));
        tab.set_view_state(DocumentTabViewState {
            first_visible_line: 3,
            caret_position_utf16: 4,
            selection_start_utf16: 4,
            selection_end_utf16: 4,
        });

        assert!(tab.set_content("draft".to_owned()));
        tab.discard_changes();

        assert_eq!(tab.content, "saved");
        assert_loaded_is_current(&tab);
        assert!(!tab.dirty);
        assert_eq!(tab.view_state, DocumentTabViewState::default());
    }

    #[test]
    fn dirty_sync_updates_token_against_saved_baseline() {
        let mut tab = OpenDocumentTab::from_input(input("saved", "v1"));
        assert!(tab.set_content("draft".to_owned()));

        tab.sync_from(input("saved", "v2"), true);
        assert_eq!(tab.content, "draft");
        assert_loaded_is_saved(&tab, "saved");
        assert_eq!(tab.loaded_updated_at, "v2");
        assert!(tab.dirty);

        tab.sync_from(input("remote", "v3"), true);
        assert_eq!(tab.content, "draft");
        assert_loaded_is_saved(&tab, "saved");
        assert_eq!(tab.loaded_updated_at, "v2");
        assert!(tab.dirty);
    }

    #[test]
    fn metadata_sync_preserves_content_dirty_state_and_save_conflict_token() {
        let mut tabs = OpenTabs::new();
        tabs.open_or_activate(input("loaded", "initial-token"));
        tabs.update_active_content("draft".to_owned());

        tabs.sync_loaded_tab_metadata(
            1,
            Some(42),
            "Renamed".to_owned(),
            true,
            DocumentTabSource::SearchResult,
        );

        let tab = tabs.active().expect("active tab should exist");
        assert_eq!(tab.parent_id, Some(42));
        assert_eq!(tab.title, "Renamed");
        assert_eq!(tab.content, "draft");
        assert_loaded_is_saved(tab, "loaded");
        assert_eq!(tab.loaded_updated_at, "initial-token");
        assert!(tab.dirty);
        assert_eq!(tab.source, DocumentTabSource::SearchResult);
    }

    #[test]
    fn indexed_metadata_sync_updates_matching_tab_only() {
        let mut tabs = OpenTabs::new();
        tabs.open_or_activate(input("loaded", "initial-token"));

        assert!(!tabs.sync_loaded_tab_metadata_at(
            0,
            2,
            Some(42),
            "Ignored".to_owned(),
            false,
            DocumentTabSource::SearchResult,
        ));
        let tab = tabs.active().expect("active tab should exist");
        assert_eq!(tab.parent_id, Some(0));
        assert_eq!(tab.title, "Title");
        assert!(tab.editable);
        assert_eq!(tab.source, DocumentTabSource::ActiveTree);

        assert!(tabs.sync_loaded_tab_metadata_at(
            0,
            1,
            Some(42),
            "Renamed".to_owned(),
            false,
            DocumentTabSource::SearchResult,
        ));
        let tab = tabs.active().expect("active tab should exist");
        assert_eq!(tab.parent_id, Some(42));
        assert_eq!(tab.title, "Renamed");
        assert_eq!(tab.content, "loaded");
        assert_eq!(tab.loaded_updated_at, "initial-token");
        assert!(!tab.editable);
        assert_eq!(tab.source, DocumentTabSource::SearchResult);
    }

    #[test]
    fn metadata_token_sync_preserves_clean_content_buffer() {
        let mut tabs = OpenTabs::new();
        tabs.open_or_activate(input("loaded", "initial-token"));
        let content_ptr = tabs
            .active()
            .expect("active tab should exist")
            .content
            .as_ptr();

        tabs.sync_loaded_tab_metadata_preserving_content(
            1,
            Some(42),
            "Renamed".to_owned(),
            "next-token".to_owned(),
            false,
            DocumentTabSource::SearchResult,
        );

        let tab = tabs.active().expect("active tab should exist");
        assert_eq!(tab.parent_id, Some(42));
        assert_eq!(tab.title, "Renamed");
        assert_eq!(tab.content, "loaded");
        assert_eq!(tab.content.as_ptr(), content_ptr);
        assert_loaded_is_current(tab);
        assert_eq!(tab.loaded_updated_at, "next-token");
        assert!(!tab.dirty);
        assert!(!tab.editable);
        assert_eq!(tab.source, DocumentTabSource::SearchResult);
    }

    #[test]
    fn metadata_token_sync_preserves_dirty_content_and_save_conflict_token() {
        let mut tabs = OpenTabs::new();
        tabs.open_or_activate(input("loaded", "initial-token"));
        tabs.update_active_content("draft".to_owned());

        tabs.sync_loaded_tab_metadata_preserving_content(
            1,
            Some(42),
            "Renamed".to_owned(),
            "next-token".to_owned(),
            true,
            DocumentTabSource::SearchResult,
        );

        let tab = tabs.active().expect("active tab should exist");
        assert_eq!(tab.parent_id, Some(42));
        assert_eq!(tab.title, "Renamed");
        assert_eq!(tab.content, "draft");
        assert_loaded_is_saved(tab, "loaded");
        assert_eq!(tab.loaded_updated_at, "initial-token");
        assert!(tab.dirty);
        assert_eq!(tab.source, DocumentTabSource::SearchResult);
    }

    #[test]
    fn indexed_metadata_token_sync_updates_dirty_token_without_replacing_content() {
        let mut tabs = OpenTabs::new();
        tabs.open_or_activate(input("loaded", "initial-token"));
        tabs.update_active_content("draft".to_owned());
        let content_ptr = tabs
            .active()
            .expect("active tab should exist")
            .content
            .as_ptr();

        assert!(tabs.sync_loaded_tab_metadata_preserving_content_at(
            0,
            LoadedTabMetadataUpdate {
                node_id: 1,
                parent_id: Some(42),
                title: "Renamed".to_owned(),
                loaded_updated_at: "next-token".to_owned(),
                editable: true,
                source: DocumentTabSource::ActiveTree,
                current_content_for_dirty_token: Some("loaded"),
            },
        ));
        let tab = tabs.active().expect("active tab should exist");
        assert_eq!(tab.content, "draft");
        assert_eq!(tab.content.as_ptr(), content_ptr);
        assert_loaded_is_saved(tab, "loaded");
        assert_eq!(tab.loaded_updated_at, "next-token");
        assert!(tab.dirty);

        assert!(tabs.sync_loaded_tab_metadata_preserving_content_at(
            0,
            LoadedTabMetadataUpdate {
                node_id: 1,
                parent_id: Some(42),
                title: "Renamed again".to_owned(),
                loaded_updated_at: "stale-token".to_owned(),
                editable: true,
                source: DocumentTabSource::ActiveTree,
                current_content_for_dirty_token: Some("remote"),
            },
        ));
        let tab = tabs.active().expect("active tab should exist");
        assert_eq!(tab.title, "Renamed again");
        assert_eq!(tab.content.as_ptr(), content_ptr);
        assert_loaded_is_saved(tab, "loaded");
        assert_eq!(tab.loaded_updated_at, "next-token");
        assert!(tab.dirty);
    }

    #[test]
    fn dirty_from_view_defers_saved_baseline_until_content_sync() {
        let mut tab = OpenDocumentTab::from_input(input("saved", "v1"));

        assert!(tab.mark_dirty_from_view());
        assert_loaded_is_current(&tab);
        assert!(tab.dirty);

        tab.mark_current_content_saved("v2".to_owned());
        assert_loaded_is_current(&tab);
        assert_eq!(tab.loaded_updated_at, "v2");
        assert!(!tab.dirty);

        assert!(tab.mark_dirty_from_view());
        assert_loaded_is_current(&tab);
        assert!(tab.dirty);

        tab.discard_changes();
        assert_loaded_is_current(&tab);
        assert_eq!(tab.content, "saved");
        assert!(!tab.dirty);

        assert!(tab.mark_dirty_from_view());
        let mut unchanged_content = String::from("saved");
        assert!(tab.set_content_reusing(&mut unchanged_content));
        assert_loaded_is_current(&tab);
        assert_eq!(tab.content, "saved");
        assert!(!tab.dirty);

        assert!(tab.mark_dirty_from_view());
        assert!(tab.set_content("draft".to_owned()));
        assert_loaded_is_saved(&tab, "saved");
        assert_eq!(tab.content, "draft");
        assert!(tab.dirty);

        assert!(tab.set_content("saved".to_owned()));
        assert_loaded_is_current(&tab);
        assert_eq!(tab.content, "saved");
        assert!(!tab.dirty);
    }

    #[test]
    fn move_tab_preserves_active_document_across_reorder() {
        let mut tabs = OpenTabs::new();
        tabs.open_or_activate(input_for_node(1, "one", "v1"));
        tabs.open_or_activate(input_for_node(2, "two", "v2"));
        tabs.open_or_activate(input_for_node(3, "three", "v3"));
        assert!(tabs.set_active(1));

        assert!(tabs.move_tab(0, 2));
        assert_eq!(tab_node_ids(&tabs), vec![2, 3, 1]);
        assert_eq!(tabs.active_index(), Some(0));
        assert_eq!(tabs.active().map(|tab| tab.node_id), Some(2));

        assert!(tabs.move_tab(0, 2));
        assert_eq!(tab_node_ids(&tabs), vec![3, 1, 2]);
        assert_eq!(tabs.active_index(), Some(2));
        assert_eq!(tabs.active().map(|tab| tab.node_id), Some(2));

        assert!(!tabs.move_tab(9, 0));
        assert!(!tabs.move_tab(0, 9));
        assert_eq!(tab_node_ids(&tabs), vec![3, 1, 2]);
        assert_eq!(tabs.active_index(), Some(2));
    }

    fn input(content: &str, loaded_updated_at: &str) -> OpenDocumentTabInput {
        input_for_node(1, content, loaded_updated_at)
    }

    fn input_for_node(
        node_id: i64,
        content: &str,
        loaded_updated_at: &str,
    ) -> OpenDocumentTabInput {
        OpenDocumentTabInput {
            node_id,
            parent_id: Some(0),
            title: "Title".to_owned(),
            content: content.to_owned(),
            loaded_updated_at: loaded_updated_at.to_owned(),
            editable: true,
            source: DocumentTabSource::ActiveTree,
        }
    }

    fn tab_node_ids(tabs: &OpenTabs) -> Vec<i64> {
        tabs.tabs().iter().map(|tab| tab.node_id).collect()
    }

    fn assert_loaded_is_current(tab: &OpenDocumentTab) {
        assert!(matches!(&tab.loaded_content, LoadedContent::Current));
        assert_eq!(tab.loaded_content(), tab.content);
    }

    fn assert_loaded_is_saved(tab: &OpenDocumentTab, expected: &str) {
        assert_eq!(tab.loaded_content(), expected);
        match &tab.loaded_content {
            LoadedContent::Saved(content) => assert_eq!(content, expected),
            LoadedContent::Current => panic!("loaded content should be saved separately"),
        }
    }
}
