use rusqlite::Connection;

use crate::domain::NodeSiblingOrderUpdate;

mod node;
mod path;
mod repository;
mod schema;
mod search;
mod settings;
mod tree;
mod tree_repository;

#[cfg(test)]
mod tests;

pub use path::{database_path_for_current_exe, database_path_for_executable};

pub struct SqliteDocumentRepository {
    connection: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamedNodeUpdate {
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteNodeUpdate {
    pub removed_node_ids: Vec<i64>,
    pub sibling_orders: Vec<NodeSiblingOrderUpdate>,
}
