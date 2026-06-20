use std::collections::{HashMap, HashSet};

use rusqlite::{params, Transaction};

use crate::domain::{DomainError, Node, ROOT_NODE_ID};
use crate::error::AppError;

use super::super::node::RestorableNodeSummary;
use super::lookup::{active_node_exists, ensure_active_node};
use super::subtree::{
    clear_cascade_subtree_workspace, metadata_timestamp_after_staged_deleted_subtree,
    stage_deleted_subtree,
};
use super::titles::{active_child_titles_by_parent, unique_restored_child_title_from_titles};

struct RestorableSubtreeMetadata {
    summary: RestorableNodeSummary,
    sort_order: i64,
    created_at: String,
}

pub(in crate::infra::sqlite) fn restore_target_parent_id(
    transaction: &Transaction<'_>,
    original_parent_id: Option<i64>,
) -> Result<Option<i64>, AppError> {
    if let Some(parent_id) = original_parent_id {
        if active_node_exists(transaction, parent_id)? {
            return Ok(Some(parent_id));
        }
    }

    ensure_active_node(transaction, ROOT_NODE_ID)?;
    Ok(Some(ROOT_NODE_ID))
}

pub(in crate::infra::sqlite) fn restore_deleted_subtree_metadata_nodes(
    transaction: &Transaction<'_>,
    node_id: i64,
    target_parent_id: Option<i64>,
    title: &str,
    sort_order: i64,
) -> Result<Vec<Node>, AppError> {
    let subtree_len = stage_deleted_subtree(transaction, node_id)?;
    if subtree_len == 0 {
        return Err(DomainError::NodeNotDeleted { node_id }.into());
    }

    let restored_at = metadata_timestamp_after_staged_deleted_subtree(transaction)?;
    update_deleted_root_for_restore(
        transaction,
        node_id,
        target_parent_id,
        title,
        sort_order,
        &restored_at,
    )?;

    let mut nodes = restorable_subtree_metadata_nodes(transaction)?;
    ensure_restored_subtree_child_titles(transaction, &mut nodes)?;
    mark_restorable_subtree_restored(transaction, node_id, &nodes, &restored_at)?;
    clear_cascade_subtree_workspace(transaction)?;

    nodes
        .into_iter()
        .map(|node| {
            let node = Node {
                id: node.summary.id,
                parent_id: node.summary.parent_id,
                title: node.summary.title,
                sort_order: node.sort_order,
                content: String::new(),
                created_at: node.created_at,
                updated_at: restored_at.clone(),
                deleted_at: None,
            };
            node.validate()?;
            Ok(node)
        })
        .collect()
}

fn update_deleted_root_for_restore(
    transaction: &Transaction<'_>,
    node_id: i64,
    target_parent_id: Option<i64>,
    title: &str,
    sort_order: i64,
    restored_at: &str,
) -> Result<(), AppError> {
    let updated_root = transaction
        .execute(
            "
        UPDATE nodes
        SET parent_id = ?1, title = ?2, sort_order = ?3, updated_at = ?4
        WHERE id = ?5 AND deleted_at IS NOT NULL
        ",
            params![target_parent_id, title, sort_order, restored_at, node_id],
        )
        .map_err(|source| AppError::sqlite("prepare deleted SQLite node for restore", source))?;

    if updated_root == 0 {
        return Err(DomainError::NodeNotDeleted { node_id }.into());
    }

    Ok(())
}

fn ensure_restored_subtree_child_titles(
    transaction: &Transaction<'_>,
    nodes: &mut [RestorableSubtreeMetadata],
) -> Result<(), AppError> {
    let mut active_titles_by_parent = active_child_titles_by_parent(
        transaction,
        nodes.iter().map(|node| node.summary.parent_id),
    )?;
    let mut reserved_titles_by_parent: HashMap<Option<i64>, HashSet<String>> = HashMap::new();

    for node in nodes {
        let active_titles = active_titles_by_parent
            .entry(node.summary.parent_id)
            .or_default();
        let reserved_titles = reserved_titles_by_parent
            .entry(node.summary.parent_id)
            .or_default();
        let title = unique_restored_child_title_from_titles(
            active_titles,
            &node.summary.title,
            reserved_titles,
        )?;

        if title != node.summary.title {
            update_deleted_node_title(transaction, node.summary.id, &title)?;
            node.summary.title = title.clone();
        }

        reserved_titles.insert(title);
    }

    Ok(())
}

fn restorable_subtree_metadata_nodes(
    transaction: &Transaction<'_>,
) -> Result<Vec<RestorableSubtreeMetadata>, AppError> {
    let mut statement = transaction
        .prepare(
            "
        SELECT nodes.id, nodes.parent_id, nodes.title, nodes.sort_order, nodes.created_at
        FROM nodes
        INNER JOIN temp.j3_tree_cascade_subtree subtree ON subtree.node_id = nodes.id
        ORDER BY
            subtree.depth,
            nodes.parent_id IS NOT NULL,
            nodes.parent_id,
            (nodes.deleted_at = subtree.root_deleted_at) DESC,
            nodes.sort_order,
            nodes.title,
            nodes.id
        ",
        )
        .map_err(|source| AppError::sqlite("prepare restorable SQLite subtree query", source))?;

    let rows = statement
        .query_map([], |row| {
            Ok(RestorableSubtreeMetadata {
                summary: RestorableNodeSummary {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    title: row.get(2)?,
                },
                sort_order: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|source| AppError::sqlite("query restorable SQLite subtree", source))?;

    let mut nodes = Vec::new();
    for row in rows {
        nodes.push(
            row.map_err(|source| AppError::sqlite("read restorable SQLite subtree row", source))?,
        );
    }

    Ok(nodes)
}

fn mark_restorable_subtree_restored(
    transaction: &Transaction<'_>,
    node_id: i64,
    nodes: &[RestorableSubtreeMetadata],
    restored_at: &str,
) -> Result<(), AppError> {
    if nodes.is_empty() {
        return Err(DomainError::NodeNotDeleted { node_id }.into());
    }

    let restored = transaction
        .execute(
            "
        UPDATE nodes
        SET deleted_at = NULL, updated_at = ?1
        WHERE id IN (
            SELECT node_id
            FROM temp.j3_tree_cascade_subtree
        )
            AND deleted_at IS NOT NULL
        ",
            params![restored_at],
        )
        .map_err(|source| AppError::sqlite("restore SQLite node subtree", source))?;

    if restored != nodes.len() {
        return Err(DomainError::NodeNotDeleted { node_id }.into());
    }

    Ok(())
}

pub(in crate::infra::sqlite) fn update_deleted_node_title(
    transaction: &Transaction<'_>,
    node_id: i64,
    title: &str,
) -> Result<(), AppError> {
    let updated = transaction
        .execute(
            "
        UPDATE nodes
        SET title = ?1
        WHERE id = ?2 AND deleted_at IS NOT NULL
        ",
            params![title, node_id],
        )
        .map_err(|source| AppError::sqlite("rename deleted SQLite node before restore", source))?;

    if updated == 0 {
        return Err(DomainError::NodeNotDeleted { node_id }.into());
    }

    Ok(())
}
