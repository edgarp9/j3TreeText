use std::error::Error;
use std::fmt;

mod document_tree;
mod open_tabs;
mod text_edit;
mod ui_settings;

#[cfg(test)]
mod tests;

pub const APP_DISPLAY_NAME: &str = "j3TreeText";
pub const APP_LINUX_APPLICATION_ID: &str = "io.github.edgarp9.j3TreeText";
pub const APP_AUTHOR_URL: &str = "https://github.com/edgarp9";
pub const APP_ICON_SVG_FILE_NAME: &str = "icon.svg";
pub const APP_ICON_PNG_FILE_NAME: &str = "icon.png";

pub use document_tree::{
    Document, DocumentSearchResult, Node, NodeSiblingOrderUpdate, SiblingMoveDirection,
    DEFAULT_DOCUMENT_ID, DEFAULT_DOCUMENT_TITLE, ROOT_NODE_ID, ROOT_TITLE, SEARCH_RESULT_LIMIT,
};

pub use open_tabs::{
    DirtyTabDecision, DocumentTabSource, DocumentTabViewState, LoadedTabMetadataUpdate,
    OpenDocumentTab, OpenDocumentTabInput, OpenTabResult, OpenTabs,
};

pub use text_edit::{
    find_next_literal, replace_all_literal, replace_literal_at, ReplaceAllError, ReplaceAllResult,
    ReplaceOneResult, TextMatch,
};

pub use ui_settings::{
    auto_save_enabled_storage_value, dark_theme_storage_value, editor_word_wrap_storage_value,
    toggle_editor_word_wrap, AppearanceSettings, AppearanceTheme, AutoSaveSettings,
    EditorFontSettings, EditorSettings, SelectionSettings, SplitterSettings, TextEncoding,
    TextEncodingSettings, UiLanguage, UiSettings, WindowSettings, DEFAULT_APPEARANCE_THEME,
    DEFAULT_AUTO_SAVE_ENABLED, DEFAULT_AUTO_SAVE_INTERVAL_SECONDS, DEFAULT_DARK_THEME,
    DEFAULT_EDITOR_FONT_FAMILY, DEFAULT_EDITOR_FONT_SIZE_PT, DEFAULT_EDITOR_WORD_WRAP,
    DEFAULT_SPLITTER_LEFT_WIDTH, DEFAULT_UI_LANGUAGE, DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH,
    MAX_EDITOR_FONT_SIZE_PT, MIN_EDITOR_FONT_SIZE_PT, SETTING_APPEARANCE_DARK_THEME,
    SETTING_APPEARANCE_THEME, SETTING_AUTO_SAVE_ENABLED, SETTING_AUTO_SAVE_INTERVAL_SECONDS,
    SETTING_EDITOR_FONT_FAMILY, SETTING_EDITOR_FONT_SIZE_PT, SETTING_EDITOR_WORD_WRAP,
    SETTING_SELECTION_NODE_ID, SETTING_SPLITTER_LEFT_WIDTH, SETTING_TEXT_EXPORT_ENCODING,
    SETTING_TEXT_IMPORT_ENCODING, SETTING_UI_LANGUAGE, SETTING_WINDOW_HEIGHT, SETTING_WINDOW_WIDTH,
    SETTING_WINDOW_X, SETTING_WINDOW_Y,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    CannotDeleteRoot,
    CannotMoveNodeIntoDescendant {
        node_id: i64,
        parent_id: i64,
    },
    CannotMoveNodeIntoItself {
        node_id: i64,
    },
    CannotMoveRoot,
    DocumentSaveConflict {
        node_id: i64,
    },
    DuplicateNodeId(i64),
    DuplicateSiblingTitle {
        parent_id: Option<i64>,
        title: String,
    },
    EmptyTitle {
        node_id: i64,
    },
    EmptyTitleInput,
    EmbeddedNulContent {
        node_id: i64,
    },
    EmbeddedNulTitle {
        node_id: i64,
    },
    EmbeddedNulTitleInput,
    InvalidNodeId(i64),
    InvalidParent {
        node_id: i64,
        parent_id: i64,
    },
    InvalidSortOrder {
        node_id: i64,
        sort_order: i64,
    },
    ParentCycle {
        node_id: i64,
    },
    MissingParent {
        node_id: i64,
        parent_id: i64,
    },
    MissingRoot,
    MissingTimestamp {
        node_id: i64,
    },
    MultipleRoots,
    NodeNotFound {
        node_id: i64,
    },
    NodeNotDeleted {
        node_id: i64,
    },
    UnreachableNode {
        node_id: i64,
    },
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CannotDeleteRoot => write!(formatter, "root document cannot be deleted"),
            Self::CannotMoveNodeIntoDescendant { node_id, parent_id } => write!(
                formatter,
                "node {node_id} cannot be moved under descendant node {parent_id}"
            ),
            Self::CannotMoveNodeIntoItself { node_id } => {
                write!(formatter, "node {node_id} cannot be moved under itself")
            }
            Self::CannotMoveRoot => write!(formatter, "root document cannot be moved"),
            Self::DocumentSaveConflict { node_id } => {
                write!(
                    formatter,
                    "document node {node_id} was changed after it was loaded"
                )
            }
            Self::DuplicateNodeId(id) => write!(formatter, "duplicate node id: {id}"),
            Self::DuplicateSiblingTitle { parent_id, title } => {
                if let Some(parent_id) = parent_id {
                    write!(
                        formatter,
                        "parent node {parent_id} already has an active child titled {title:?}"
                    )
                } else {
                    write!(
                        formatter,
                        "root level already has an active node titled {title:?}"
                    )
                }
            }
            Self::EmptyTitle { node_id } => write!(formatter, "node {node_id} has an empty title"),
            Self::EmptyTitleInput => write!(formatter, "node title input is empty"),
            Self::EmbeddedNulContent { node_id } => {
                write!(
                    formatter,
                    "node {node_id} content contains an embedded NUL character"
                )
            }
            Self::EmbeddedNulTitle { node_id } => {
                write!(
                    formatter,
                    "node {node_id} title contains an embedded NUL character"
                )
            }
            Self::EmbeddedNulTitleInput => {
                write!(
                    formatter,
                    "node title input contains an embedded NUL character"
                )
            }
            Self::InvalidNodeId(id) => write!(formatter, "invalid node id: {id}"),
            Self::InvalidParent { node_id, parent_id } => write!(
                formatter,
                "node {node_id} has invalid parent id {parent_id}"
            ),
            Self::InvalidSortOrder {
                node_id,
                sort_order,
            } => write!(
                formatter,
                "node {node_id} has invalid sort_order {sort_order}"
            ),
            Self::MissingParent { node_id, parent_id } => write!(
                formatter,
                "node {node_id} references missing parent {parent_id}"
            ),
            Self::MissingRoot => write!(formatter, "document has no root node"),
            Self::MissingTimestamp { node_id } => {
                write!(formatter, "node {node_id} is missing timestamps")
            }
            Self::MultipleRoots => write!(formatter, "document has multiple root nodes"),
            Self::NodeNotFound { node_id } => {
                write!(formatter, "active node {node_id} was not found")
            }
            Self::NodeNotDeleted { node_id } => {
                write!(formatter, "node {node_id} is not in the trash")
            }
            Self::ParentCycle { node_id } => {
                write!(
                    formatter,
                    "document tree has a parent cycle at node {node_id}"
                )
            }
            Self::UnreachableNode { node_id } => {
                write!(formatter, "node {node_id} is not reachable from the root")
            }
        }
    }
}

impl Error for DomainError {}
