use rusqlite::{params, Params, Transaction};

use crate::domain::DomainError;
use crate::error::AppError;

pub(in crate::infra::sqlite) fn active_node_has_ancestor(
    transaction: &Transaction<'_>,
    node_id: i64,
    ancestor_id: i64,
) -> Result<bool, AppError> {
    let count: i64 = transaction
        .query_row(
            "
        WITH RECURSIVE ancestors(id, parent_id) AS (
            SELECT id, parent_id
            FROM nodes
            WHERE id = ?1 AND deleted_at IS NULL

            UNION ALL

            SELECT parent.id, parent.parent_id
            FROM nodes parent
            INNER JOIN ancestors child ON child.parent_id = parent.id
            WHERE parent.deleted_at IS NULL
        )
        SELECT COUNT(*)
        FROM ancestors
        WHERE id = ?2
        ",
            params![node_id, ancestor_id],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("check SQLite node ancestry", source))?;

    Ok(count > 0)
}

pub(in crate::infra::sqlite) fn soft_delete_active_subtree_node_ids(
    transaction: &Transaction<'_>,
    node_id: i64,
    parent_id: Option<i64>,
) -> Result<(String, Vec<i64>), AppError> {
    let subtree_len = stage_active_subtree(transaction, node_id)?;
    if subtree_len == 0 {
        return Err(DomainError::NodeNotFound { node_id }.into());
    }

    soft_delete_staged_active_subtree_node_ids(transaction, node_id, parent_id)
}

pub(in crate::infra::sqlite) fn soft_delete_staged_active_subtree_node_ids(
    transaction: &Transaction<'_>,
    node_id: i64,
    parent_id: Option<i64>,
) -> Result<(String, Vec<i64>), AppError> {
    let node_ids = staged_subtree_node_ids(transaction)?;
    if node_ids.is_empty() {
        return Err(DomainError::NodeNotFound { node_id }.into());
    }

    let deleted_at =
        metadata_timestamp_after_staged_active_subtree_and_siblings(transaction, parent_id)?;
    let updated = transaction
        .execute(
            "
        UPDATE nodes
        SET deleted_at = ?1, updated_at = ?1
        WHERE id IN (
            SELECT node_id
            FROM temp.j3_tree_cascade_subtree
        )
            AND deleted_at IS NULL
        ",
            params![&deleted_at],
        )
        .map_err(|source| AppError::sqlite("soft delete SQLite subtree", source))?;

    if updated != node_ids.len() {
        return Err(AppError::internal_consistency(
            "soft delete SQLite subtree",
            "updated row count did not match staged subtree row count",
        ));
    }

    clear_cascade_subtree_workspace(transaction)?;

    Ok((deleted_at, node_ids))
}

pub(in crate::infra::sqlite) fn staged_active_subtree_matches(
    transaction: &Transaction<'_>,
    node_id: i64,
    expected_node_ids: &[i64],
) -> Result<bool, AppError> {
    ensure_cascade_subtree_workspace(transaction)?;
    if expected_node_ids.is_empty() {
        return Ok(false);
    }

    let staged_node_ids = staged_subtree_node_ids(transaction)?;
    if staged_node_ids != expected_node_ids || !staged_node_ids.contains(&node_id) {
        return Ok(false);
    }

    let staged_count = i64::try_from(staged_node_ids.len()).map_err(|_| {
        AppError::internal_consistency(
            "validate staged SQLite subtree",
            "staged subtree length exceeded SQLite count range",
        )
    })?;

    let active_count: i64 = transaction
        .query_row(
            "
        SELECT COUNT(*)
        FROM nodes
        INNER JOIN temp.j3_tree_cascade_subtree subtree ON subtree.node_id = nodes.id
        WHERE nodes.deleted_at IS NULL
        ",
            [],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("count active staged SQLite subtree", source))?;
    if active_count != staged_count {
        return Ok(false);
    }

    let detached_count: i64 = transaction
        .query_row(
            "
        SELECT COUNT(*)
        FROM temp.j3_tree_cascade_subtree subtree
        INNER JOIN nodes ON nodes.id = subtree.node_id
        LEFT JOIN temp.j3_tree_cascade_subtree parent_subtree
            ON parent_subtree.node_id = nodes.parent_id
        WHERE subtree.node_id <> ?1
            AND parent_subtree.node_id IS NULL
        ",
            params![node_id],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("validate staged SQLite subtree parents", source))?;
    if detached_count != 0 {
        return Ok(false);
    }

    let extra_active_child_count: i64 = transaction
        .query_row(
            "
        SELECT COUNT(*)
        FROM nodes child
        INNER JOIN temp.j3_tree_cascade_subtree parent_subtree
            ON parent_subtree.node_id = child.parent_id
        LEFT JOIN temp.j3_tree_cascade_subtree child_subtree
            ON child_subtree.node_id = child.id
        WHERE child.deleted_at IS NULL
            AND child_subtree.node_id IS NULL
        ",
            [],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("validate staged SQLite subtree children", source))?;

    Ok(extra_active_child_count == 0)
}

pub(super) fn stage_deleted_subtree(
    transaction: &Transaction<'_>,
    node_id: i64,
) -> Result<usize, AppError> {
    reset_cascade_subtree_workspace(transaction)?;
    transaction
        .execute(
            "
        WITH RECURSIVE subtree(id, depth, root_deleted_at) AS (
            SELECT id, 0, deleted_at
            FROM nodes
            WHERE id = ?1 AND deleted_at IS NOT NULL

            UNION ALL

            SELECT child.id, subtree.depth + 1, subtree.root_deleted_at
            FROM nodes child
            INNER JOIN subtree ON child.parent_id = subtree.id
            WHERE child.deleted_at IS NOT NULL
        )
        INSERT INTO temp.j3_tree_cascade_subtree (node_id, depth, root_deleted_at)
        SELECT id, depth, root_deleted_at
        FROM subtree
        ",
            params![node_id],
        )
        .map_err(|source| AppError::sqlite("stage deleted SQLite subtree", source))
}

pub(in crate::infra::sqlite) fn staged_subtree_node_ids(
    transaction: &Transaction<'_>,
) -> Result<Vec<i64>, AppError> {
    let mut statement = transaction
        .prepare(
            "
        SELECT node_id
        FROM temp.j3_tree_cascade_subtree
        ORDER BY depth, node_id
        ",
        )
        .map_err(|source| AppError::sqlite("prepare staged SQLite subtree query", source))?;

    let rows = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|source| AppError::sqlite("query staged SQLite subtree", source))?;

    let mut node_ids = Vec::new();
    for row in rows {
        node_ids.push(
            row.map_err(|source| AppError::sqlite("read staged SQLite subtree row", source))?,
        );
    }

    Ok(node_ids)
}

pub(super) fn clear_cascade_subtree_workspace(
    transaction: &Transaction<'_>,
) -> Result<(), AppError> {
    transaction
        .execute(
            "
        DELETE FROM temp.j3_tree_cascade_subtree
        ",
            [],
        )
        .map(|_| ())
        .map_err(|source| AppError::sqlite("clear staged SQLite subtree", source))
}

pub(in crate::infra::sqlite) fn stage_active_subtree(
    transaction: &Transaction<'_>,
    node_id: i64,
) -> Result<usize, AppError> {
    reset_cascade_subtree_workspace(transaction)?;
    transaction
        .execute(
            "
        WITH RECURSIVE subtree(id, depth) AS (
            SELECT id, 0
            FROM nodes
            WHERE id = ?1 AND deleted_at IS NULL

            UNION ALL

            SELECT child.id, subtree.depth + 1
            FROM nodes child
            INNER JOIN subtree ON child.parent_id = subtree.id
            WHERE child.deleted_at IS NULL
        )
        INSERT INTO temp.j3_tree_cascade_subtree (node_id, depth, root_deleted_at)
        SELECT id, depth, NULL
        FROM subtree
        ",
            params![node_id],
        )
        .map_err(|source| AppError::sqlite("stage active SQLite subtree", source))
}

fn reset_cascade_subtree_workspace(transaction: &Transaction<'_>) -> Result<(), AppError> {
    ensure_cascade_subtree_workspace(transaction)?;
    clear_cascade_subtree_workspace(transaction)
}

fn ensure_cascade_subtree_workspace(transaction: &Transaction<'_>) -> Result<(), AppError> {
    transaction
        .execute(
            "
        CREATE TEMP TABLE IF NOT EXISTS j3_tree_cascade_subtree (
            node_id INTEGER PRIMARY KEY,
            depth INTEGER NOT NULL,
            root_deleted_at TEXT
        )
        ",
            [],
        )
        .map(|_| ())
        .map_err(|source| AppError::sqlite("create staged SQLite subtree workspace", source))
}

fn metadata_timestamp_after_staged_active_subtree_and_siblings(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
) -> Result<String, AppError> {
    metadata_timestamp_after_max_updated_at(
        transaction,
        "
        SELECT MAX(updated_at)
        FROM nodes
        WHERE deleted_at IS NULL
            AND (
                id IN (
                    SELECT node_id
                    FROM temp.j3_tree_cascade_subtree
                )
                OR parent_id IS ?1
            )
        ",
        params![parent_id],
        "read staged active SQLite subtree timestamps for metadata delete",
    )
}

pub(super) fn metadata_timestamp_after_staged_deleted_subtree(
    transaction: &Transaction<'_>,
) -> Result<String, AppError> {
    metadata_timestamp_after_max_updated_at(
        transaction,
        "
        SELECT MAX(nodes.updated_at)
        FROM nodes
        INNER JOIN temp.j3_tree_cascade_subtree subtree ON subtree.node_id = nodes.id
        WHERE nodes.deleted_at IS NOT NULL
        ",
        [],
        "read staged deleted SQLite subtree timestamps for metadata restore",
    )
}

fn metadata_timestamp_after_max_updated_at<P>(
    transaction: &Transaction<'_>,
    sql: &str,
    params: P,
    context: &'static str,
) -> Result<String, AppError>
where
    P: Params,
{
    let previous_updated_at = transaction
        .query_row(sql, params, |row| row.get::<_, Option<String>>(0))
        .map_err(|source| AppError::sqlite(context, source))?;

    match previous_updated_at {
        Some(previous_updated_at) => {
            super::super::node::next_timestamp_after(transaction, &previous_updated_at)
        }
        None => super::super::node::current_timestamp(transaction),
    }
}
