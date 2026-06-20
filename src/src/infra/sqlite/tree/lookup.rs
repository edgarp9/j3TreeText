use rusqlite::{params, OptionalExtension, Transaction};

use crate::domain::{DomainError, ROOT_NODE_ID};
use crate::error::AppError;

use super::super::node::{ActiveNodeSummary, DeletedNodeSummary};
use super::subtree::active_node_has_ancestor;

pub(in crate::infra::sqlite) fn ensure_active_node(
    transaction: &Transaction<'_>,
    node_id: i64,
) -> Result<(), AppError> {
    let exists: bool = transaction
        .query_row(
            "
        SELECT EXISTS (
            SELECT 1
            FROM nodes
            WHERE id = ?1 AND deleted_at IS NULL
        )
        ",
            params![node_id],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("check active SQLite node", source))?;

    if exists {
        Ok(())
    } else {
        Err(DomainError::NodeNotFound { node_id }.into())
    }
}

pub(in crate::infra::sqlite) fn ensure_movable_node(
    node_id: i64,
    parent_id: Option<i64>,
) -> Result<(), AppError> {
    if node_id == ROOT_NODE_ID || parent_id.is_none() {
        return Err(DomainError::CannotMoveRoot.into());
    }

    Ok(())
}

pub(in crate::infra::sqlite) fn ensure_node_can_move_to_parent(
    transaction: &Transaction<'_>,
    node_id: i64,
    parent_id: i64,
) -> Result<(), AppError> {
    if node_id == parent_id {
        return Err(DomainError::CannotMoveNodeIntoItself { node_id }.into());
    }

    if active_node_has_ancestor(transaction, parent_id, node_id)? {
        return Err(DomainError::CannotMoveNodeIntoDescendant { node_id, parent_id }.into());
    }

    Ok(())
}

pub(in crate::infra::sqlite) fn active_node_summary(
    transaction: &Transaction<'_>,
    node_id: i64,
) -> Result<ActiveNodeSummary, AppError> {
    let record: Option<(Option<i64>, String, i64)> = transaction
        .query_row(
            "
        SELECT parent_id, title, sort_order
        FROM nodes
        WHERE id = ?1 AND deleted_at IS NULL
        ",
            params![node_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|source| AppError::sqlite("read active SQLite node", source))?;

    let Some((parent_id, title, sort_order)) = record else {
        return Err(DomainError::NodeNotFound { node_id }.into());
    };

    Ok(ActiveNodeSummary {
        parent_id,
        title,
        sort_order,
    })
}

pub(in crate::infra::sqlite) fn active_node_parent_id(
    transaction: &Transaction<'_>,
    node_id: i64,
) -> Result<Option<i64>, AppError> {
    let parent_id: Option<Option<i64>> = transaction
        .query_row(
            "
        SELECT parent_id
        FROM nodes
        WHERE id = ?1 AND deleted_at IS NULL
        ",
            params![node_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|source| AppError::sqlite("read active SQLite node parent", source))?;

    parent_id.ok_or_else(|| DomainError::NodeNotFound { node_id }.into())
}

pub(in crate::infra::sqlite) fn active_node_parent_sort_order(
    transaction: &Transaction<'_>,
    node_id: i64,
) -> Result<(Option<i64>, i64), AppError> {
    let record: Option<(Option<i64>, i64)> = transaction
        .query_row(
            "
        SELECT parent_id, sort_order
        FROM nodes
        WHERE id = ?1 AND deleted_at IS NULL
        ",
            params![node_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|source| AppError::sqlite("read active SQLite node sort order", source))?;

    record.ok_or_else(|| DomainError::NodeNotFound { node_id }.into())
}

pub(in crate::infra::sqlite) fn deleted_node_summary(
    transaction: &Transaction<'_>,
    node_id: i64,
) -> Result<DeletedNodeSummary, AppError> {
    let record: Option<(Option<i64>, String)> = transaction
        .query_row(
            "
        SELECT parent_id, title
        FROM nodes
        WHERE id = ?1 AND deleted_at IS NOT NULL
        ",
            params![node_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|source| AppError::sqlite("read deleted SQLite node", source))?;

    let Some((parent_id, title)) = record else {
        return Err(DomainError::NodeNotDeleted { node_id }.into());
    };

    Ok(DeletedNodeSummary { parent_id, title })
}

pub(in crate::infra::sqlite) fn ensure_deleted_node(
    transaction: &Transaction<'_>,
    node_id: i64,
) -> Result<(), AppError> {
    let exists: bool = transaction
        .query_row(
            "
        SELECT EXISTS (
            SELECT 1
            FROM nodes
            WHERE id = ?1 AND deleted_at IS NOT NULL
        )
        ",
            params![node_id],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("check deleted SQLite node", source))?;

    if exists {
        Ok(())
    } else {
        Err(DomainError::NodeNotDeleted { node_id }.into())
    }
}

pub(in crate::infra::sqlite) fn active_node_exists(
    transaction: &Transaction<'_>,
    node_id: i64,
) -> Result<bool, AppError> {
    transaction
        .query_row(
            "
        SELECT EXISTS (
            SELECT 1
            FROM nodes
            WHERE id = ?1 AND deleted_at IS NULL
        )
        ",
            params![node_id],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("check active SQLite node", source))
}

pub(in crate::infra::sqlite) fn next_node_id(
    transaction: &Transaction<'_>,
) -> Result<i64, AppError> {
    let max_id: Option<i64> = transaction
        .query_row("SELECT MAX(id) FROM nodes", [], |row| row.get(0))
        .map_err(|source| AppError::sqlite("read max SQLite node id", source))?;

    let Some(max_id) = max_id else {
        return Ok(1);
    };

    max_id.checked_add(1).ok_or_else(|| {
        AppError::internal_consistency(
            "calculate next SQLite node id",
            "SQLite node id reached i64::MAX",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn create_lookup_connection() -> Result<Connection, AppError> {
        let connection = Connection::open_in_memory()
            .map_err(|source| AppError::sqlite("open in-memory SQLite database", source))?;
        connection
            .execute_batch("CREATE TABLE nodes (id INTEGER PRIMARY KEY);")
            .map_err(|source| AppError::sqlite("create test SQLite nodes table", source))?;
        Ok(connection)
    }

    #[test]
    fn next_node_id_returns_app_error_when_max_id_is_i64_max() -> Result<(), AppError> {
        let mut connection = create_lookup_connection()?;
        connection
            .execute("INSERT INTO nodes (id) VALUES (?1)", params![i64::MAX])
            .map_err(|source| AppError::sqlite("insert test SQLite node", source))?;

        let transaction = connection
            .transaction()
            .map_err(|source| AppError::sqlite("start test SQLite transaction", source))?;

        match next_node_id(&transaction) {
            Err(AppError::User { .. }) => Ok(()),
            Err(AppError::Sqlite { .. }) => Err(AppError::user(
                "next_node_id returned a SQLite error instead of an app error",
            )),
            Err(error) => Err(error),
            Ok(id) => Err(AppError::user(format!(
                "next_node_id returned {id} instead of an overflow error"
            ))),
        }
    }
}
