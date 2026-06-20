use rusqlite::{params, Transaction};

use crate::domain::Node;
use crate::error::AppError;

pub(super) struct NodeRecord {
    pub(super) id: i64,
    pub(super) parent_id: Option<i64>,
    pub(super) title: String,
    pub(super) sort_order: i64,
    pub(super) content: String,
    pub(super) created_at: String,
    pub(super) updated_at: String,
    pub(super) deleted_at: Option<String>,
}

pub(super) struct ActiveNodeSummary {
    pub(super) parent_id: Option<i64>,
    pub(super) title: String,
    pub(super) sort_order: i64,
}

pub(super) struct AdjacentSibling {
    pub(super) node_id: i64,
    pub(super) sort_order: i64,
}

pub(super) struct DeletedNodeSummary {
    pub(super) parent_id: Option<i64>,
    pub(super) title: String,
}

pub(super) struct RestorableNodeSummary {
    pub(super) id: i64,
    pub(super) parent_id: Option<i64>,
    pub(super) title: String,
}

pub(super) fn node_from_record(record: NodeRecord) -> Result<Node, AppError> {
    let node = Node {
        id: record.id,
        parent_id: record.parent_id,
        title: record.title,
        sort_order: record.sort_order,
        content: record.content,
        created_at: record.created_at,
        updated_at: record.updated_at,
        deleted_at: record.deleted_at,
    };
    node.validate()?;
    Ok(node)
}

pub(super) fn current_timestamp(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<String, AppError> {
    transaction
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })
        .map_err(|source| AppError::sqlite("read SQLite current timestamp", source))
}

pub(super) fn next_timestamp_after(
    transaction: &Transaction<'_>,
    previous_timestamp: &str,
) -> Result<String, AppError> {
    let now = current_timestamp(transaction)?;
    if now.as_str() > previous_timestamp {
        return Ok(now);
    }

    transaction
        .query_row(
            "
            SELECT COALESCE(
                strftime('%Y-%m-%dT%H:%M:%fZ', ?1, '+0.001 seconds'),
                ?2
            )
            ",
            params![previous_timestamp, now],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("advance SQLite timestamp", source))
}

pub(super) fn insert_node(
    transaction: &rusqlite::Transaction<'_>,
    node: &Node,
) -> Result<(), AppError> {
    transaction
        .execute(
            "
            INSERT INTO nodes (
                id, parent_id, title, sort_order, content, created_at, updated_at, deleted_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ",
            params![
                node.id,
                node.parent_id,
                &node.title,
                node.sort_order,
                &node.content,
                &node.created_at,
                &node.updated_at,
                node.deleted_at.as_deref()
            ],
        )
        .map(|_| ())
        .map_err(|source| AppError::sqlite("insert SQLite node", source))
}
