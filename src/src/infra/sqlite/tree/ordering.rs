use rusqlite::{params, MappedRows, OptionalExtension, Row, Transaction};

use crate::domain::{DomainError, NodeSiblingOrderUpdate, SiblingMoveDirection};
use crate::error::AppError;

use super::super::node::AdjacentSibling;

struct CurrentParentEndStatus {
    has_later_sibling: bool,
    end_sort_order: Option<i64>,
}

struct SiblingOrderUpdateRow {
    title: String,
    update: NodeSiblingOrderUpdate,
}

pub(in crate::infra::sqlite) fn active_child_ids_ordered(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
) -> Result<Vec<i64>, AppError> {
    active_child_ids_ordered_without(transaction, parent_id, None)
}

pub(in crate::infra::sqlite) fn active_child_ids_ordered_without(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    excluded_node_id: Option<i64>,
) -> Result<Vec<i64>, AppError> {
    let mut statement = transaction
        .prepare(
            "
        SELECT id
        FROM nodes
        WHERE deleted_at IS NULL
            AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
            AND (?2 IS NULL OR id <> ?2)
        ORDER BY sort_order, title, id
        ",
        )
        .map_err(|source| AppError::sqlite("prepare SQLite sibling order query", source))?;

    let rows = statement
        .query_map(params![parent_id, excluded_node_id], |row| row.get(0))
        .map_err(|source| AppError::sqlite("query SQLite sibling order", source))?;

    let mut node_ids = Vec::new();
    for row in rows {
        node_ids.push(row.map_err(|source| AppError::sqlite("read SQLite sibling row", source))?);
    }

    Ok(node_ids)
}

pub(in crate::infra::sqlite) fn adjacent_active_sibling(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    node_id: i64,
    title: &str,
    sort_order: i64,
    direction: &SiblingMoveDirection,
) -> Result<Option<AdjacentSibling>, AppError> {
    let sql = match direction {
        SiblingMoveDirection::Up => {
            "
        SELECT id, sort_order
        FROM nodes
        WHERE deleted_at IS NULL
            AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
            AND id <> ?2
            AND (
                sort_order < ?3
                OR (
                    sort_order = ?3
                    AND (title < ?4 OR (title = ?4 AND id < ?2))
                )
            )
        ORDER BY sort_order DESC, title DESC, id DESC
        LIMIT 1
        "
        }
        SiblingMoveDirection::Down => {
            "
        SELECT id, sort_order
        FROM nodes
        WHERE deleted_at IS NULL
            AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
            AND id <> ?2
            AND (
                sort_order > ?3
                OR (
                    sort_order = ?3
                    AND (title > ?4 OR (title = ?4 AND id > ?2))
                )
            )
        ORDER BY sort_order, title, id
        LIMIT 1
        "
        }
    };

    transaction
        .query_row(sql, params![parent_id, node_id, sort_order, title], |row| {
            Ok(AdjacentSibling {
                node_id: row.get(0)?,
                sort_order: row.get(1)?,
            })
        })
        .optional()
        .map_err(|source| AppError::sqlite("read adjacent SQLite sibling", source))
}

pub(in crate::infra::sqlite) fn move_node_to_current_parent_end(
    transaction: &Transaction<'_>,
    node_id: i64,
    parent_id: Option<i64>,
    title: &str,
    current_sort_order: i64,
    updated_at: &str,
) -> Result<Vec<NodeSiblingOrderUpdate>, AppError> {
    let end_status =
        current_parent_end_status(transaction, parent_id, node_id, title, current_sort_order)?;
    if !end_status.has_later_sibling {
        return Ok(Vec::new());
    }

    let Some(end_sort_order) = end_status.end_sort_order else {
        return Err(AppError::internal_consistency(
            "move SQLite node to parent end",
            "active node is missing from its sibling set",
        ));
    };

    if current_sort_order == end_sort_order {
        let mut sibling_order = active_child_ids_ordered(transaction, parent_id)?;
        let Some(current_index) = sibling_order.iter().position(|id| *id == node_id) else {
            return Err(DomainError::NodeNotFound { node_id }.into());
        };
        let moved_node_id = sibling_order.remove(current_index);
        sibling_order.push(moved_node_id);
        recalculate_sibling_sort_order(transaction, parent_id, &sibling_order, updated_at)?;
        return sibling_order_updates(parent_id, &sibling_order, updated_at);
    }

    let mut updates = shift_sibling_sort_orders_after_removal(
        transaction,
        parent_id,
        current_sort_order,
        Some(node_id),
        updated_at,
    )?;
    update_sibling_sort_order(transaction, node_id, parent_id, end_sort_order, updated_at)?;
    updates.push(NodeSiblingOrderUpdate {
        node_id,
        parent_id,
        sort_order: end_sort_order,
        updated_at: updated_at.to_owned(),
    });

    Ok(updates)
}

fn current_parent_end_status(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    node_id: i64,
    title: &str,
    sort_order: i64,
) -> Result<CurrentParentEndStatus, AppError> {
    let (node_exists, has_later_sibling, end_sort_order): (bool, bool, Option<i64>) = transaction
        .query_row(
            "
        SELECT
            EXISTS (
                SELECT 1
                FROM nodes
                WHERE id = ?2
                    AND deleted_at IS NULL
                    AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
            ),
            EXISTS (
                SELECT 1
                FROM nodes
                WHERE deleted_at IS NULL
                    AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
                    AND id <> ?2
                    AND (
                        sort_order > ?3
                        OR (
                            sort_order = ?3
                            AND (title > ?4 OR (title = ?4 AND id > ?2))
                        )
                    )
            ),
            (
                SELECT MAX(sort_order)
                FROM nodes
                WHERE deleted_at IS NULL
                    AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
            )
        ",
            params![parent_id, node_id, sort_order, title],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|source| AppError::sqlite("read SQLite current parent end status", source))?;

    if !node_exists {
        return Err(DomainError::NodeNotFound { node_id }.into());
    }

    Ok(CurrentParentEndStatus {
        has_later_sibling,
        end_sort_order,
    })
}

pub(in crate::infra::sqlite) fn update_node_parent(
    transaction: &Transaction<'_>,
    node_id: i64,
    parent_id: Option<i64>,
    sort_order: i64,
    updated_at: &str,
) -> Result<(), AppError> {
    let updated = transaction
        .execute(
            "
        UPDATE nodes
        SET parent_id = ?1, sort_order = ?2, updated_at = ?3
        WHERE id = ?4 AND deleted_at IS NULL
        ",
            params![parent_id, sort_order, updated_at, node_id],
        )
        .map_err(|source| AppError::sqlite("update SQLite node parent", source))?;

    if updated == 0 {
        return Err(DomainError::NodeNotFound { node_id }.into());
    }

    Ok(())
}

pub(in crate::infra::sqlite) fn update_sibling_sort_order(
    transaction: &Transaction<'_>,
    node_id: i64,
    parent_id: Option<i64>,
    sort_order: i64,
    updated_at: &str,
) -> Result<(), AppError> {
    let updated = transaction
        .execute(
            "
        UPDATE nodes
        SET sort_order = ?1, updated_at = ?2
        WHERE id = ?3
            AND deleted_at IS NULL
            AND ((parent_id IS NULL AND ?4 IS NULL) OR parent_id = ?4)
        ",
            params![sort_order, updated_at, node_id, parent_id],
        )
        .map_err(|source| AppError::sqlite("update SQLite sibling sort order", source))?;

    if updated == 0 {
        return Err(DomainError::NodeNotFound { node_id }.into());
    }

    Ok(())
}

pub(in crate::infra::sqlite) fn reorder_duplicate_sibling_sort_order(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    node_id: i64,
    neighbor_node_id: i64,
    sort_order: i64,
    direction: &SiblingMoveDirection,
    updated_at: &str,
) -> Result<Vec<NodeSiblingOrderUpdate>, AppError> {
    let direction_delta = match direction {
        SiblingMoveDirection::Up => -1_i64,
        SiblingMoveDirection::Down => 1_i64,
    };
    let duplicate_count = active_sibling_sort_order_count(transaction, parent_id, sort_order)?;
    if duplicate_count < 2 {
        return Err(AppError::internal_consistency(
            "reorder duplicate SQLite sibling sort order",
            "duplicate sibling reorder requires at least two siblings with the same sort order",
        ));
    }

    let shift_delta = duplicate_count - 1;
    let Some(last_duplicate_sort_order) = sort_order.checked_add(shift_delta) else {
        return recalculate_reordered_sibling_sort_order(
            transaction,
            parent_id,
            node_id,
            neighbor_node_id,
            direction_delta,
            updated_at,
        );
    };

    let mut update_rows = Vec::new();
    if active_sibling_sort_order_exists_between(
        transaction,
        parent_id,
        sort_order + 1,
        last_duplicate_sort_order,
    )? {
        if active_sibling_max_sort_order(transaction, parent_id)?
            .and_then(|max_sort_order| max_sort_order.checked_add(shift_delta))
            .is_none()
        {
            return recalculate_reordered_sibling_sort_order(
                transaction,
                parent_id,
                node_id,
                neighbor_node_id,
                direction_delta,
                updated_at,
            );
        }
        update_rows.extend(shift_sibling_sort_orders_after_duplicate_bucket(
            transaction,
            parent_id,
            sort_order,
            shift_delta,
            updated_at,
        )?);
    }

    update_rows.extend(update_duplicate_sibling_sort_order_bucket(
        transaction,
        parent_id,
        sort_order,
        node_id,
        neighbor_node_id,
        direction_delta,
        updated_at,
    )?);

    if !update_rows.iter().any(|row| row.update.node_id == node_id) {
        return Err(DomainError::NodeNotFound { node_id }.into());
    }
    if !update_rows
        .iter()
        .any(|row| row.update.node_id == neighbor_node_id)
    {
        return Err(DomainError::NodeNotFound {
            node_id: neighbor_node_id,
        }
        .into());
    }

    Ok(sorted_sibling_order_updates(update_rows))
}

fn active_sibling_sort_order_count(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    sort_order: i64,
) -> Result<i64, AppError> {
    transaction
        .query_row(
            "
        SELECT COUNT(*)
        FROM nodes
        WHERE deleted_at IS NULL
            AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
            AND sort_order = ?2
        ",
            params![parent_id, sort_order],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("count duplicate SQLite sibling sort order", source))
}

fn active_sibling_sort_order_exists_between(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    lower_sort_order: i64,
    upper_sort_order: i64,
) -> Result<bool, AppError> {
    if lower_sort_order > upper_sort_order {
        return Ok(false);
    }

    transaction
        .query_row(
            "
        SELECT EXISTS (
            SELECT 1
            FROM nodes
            WHERE deleted_at IS NULL
                AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
                AND sort_order BETWEEN ?2 AND ?3
        )
        ",
            params![parent_id, lower_sort_order, upper_sort_order],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("check SQLite sibling sort order gap", source))
}

fn active_sibling_max_sort_order(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
) -> Result<Option<i64>, AppError> {
    transaction
        .query_row(
            "
        SELECT MAX(sort_order)
        FROM nodes
        WHERE deleted_at IS NULL
            AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
        ",
            params![parent_id],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("read max SQLite sibling sort order", source))
}

fn shift_sibling_sort_orders_after_duplicate_bucket(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    sort_order: i64,
    shift_delta: i64,
    updated_at: &str,
) -> Result<Vec<SiblingOrderUpdateRow>, AppError> {
    let mut statement = transaction
        .prepare(
            "
        UPDATE nodes
        SET sort_order = sort_order + ?1, updated_at = ?2
        WHERE deleted_at IS NULL
            AND ((parent_id IS NULL AND ?3 IS NULL) OR parent_id = ?3)
            AND sort_order > ?4
        RETURNING id, sort_order, title
        ",
        )
        .map_err(|source| AppError::sqlite("prepare shifted SQLite sibling update", source))?;

    let rows = statement
        .query_map(
            params![shift_delta, updated_at, parent_id, sort_order],
            |row| sibling_order_update_row_from_row(row, parent_id, updated_at),
        )
        .map_err(|source| AppError::sqlite("shift SQLite sibling sort order", source))?;

    collect_sibling_order_update_rows(rows, "read shifted SQLite sibling row")
}

fn update_duplicate_sibling_sort_order_bucket(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    sort_order: i64,
    node_id: i64,
    neighbor_node_id: i64,
    direction_delta: i64,
    updated_at: &str,
) -> Result<Vec<SiblingOrderUpdateRow>, AppError> {
    let mut statement = transaction
        .prepare(
            "
        WITH current_order AS (
            SELECT
                id,
                ROW_NUMBER() OVER (ORDER BY title, id) - 1 AS current_pos
            FROM nodes
            WHERE deleted_at IS NULL
                AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
                AND sort_order = ?2
        ),
        swapped_order AS (
            SELECT
                id,
                CASE
                    WHEN id = ?3 THEN current_pos + ?5
                    WHEN id = ?4 THEN current_pos - ?5
                    ELSE current_pos
                END AS swapped_pos
            FROM current_order
        ),
        new_order AS (
            SELECT
                id,
                ?2 + ROW_NUMBER() OVER (ORDER BY swapped_pos) - 1 AS new_sort_order
            FROM swapped_order
        ),
        changed_order AS (
            SELECT id, new_sort_order
            FROM new_order
            WHERE id IN (?3, ?4) OR new_sort_order <> ?2
        )
        UPDATE nodes
        SET sort_order = (
                SELECT changed_order.new_sort_order
                FROM changed_order
                WHERE changed_order.id = nodes.id
            ),
            updated_at = ?6
        WHERE id IN (SELECT id FROM changed_order)
        RETURNING id, sort_order, title
        ",
        )
        .map_err(|source| AppError::sqlite("prepare duplicate SQLite sibling update", source))?;

    let rows = statement
        .query_map(
            params![
                parent_id,
                sort_order,
                node_id,
                neighbor_node_id,
                direction_delta,
                updated_at
            ],
            |row| sibling_order_update_row_from_row(row, parent_id, updated_at),
        )
        .map_err(|source| AppError::sqlite("update duplicate SQLite sibling sort order", source))?;

    collect_sibling_order_update_rows(rows, "read duplicate SQLite sibling row")
}

fn recalculate_reordered_sibling_sort_order(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    node_id: i64,
    neighbor_node_id: i64,
    direction_delta: i64,
    updated_at: &str,
) -> Result<Vec<NodeSiblingOrderUpdate>, AppError> {
    let mut statement = transaction
        .prepare(
            "
        WITH current_order AS (
            SELECT
                id,
                ROW_NUMBER() OVER (ORDER BY sort_order, title, id) - 1 AS current_pos
            FROM nodes
            WHERE deleted_at IS NULL
                AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
        ),
        swapped_order AS (
            SELECT
                id,
                CASE
                    WHEN id = ?2 THEN current_pos + ?4
                    WHEN id = ?3 THEN current_pos - ?4
                    ELSE current_pos
                END AS swapped_pos
            FROM current_order
        ),
        new_order AS (
            SELECT
                id,
                ROW_NUMBER() OVER (ORDER BY swapped_pos) - 1 AS new_sort_order
            FROM swapped_order
        )
        UPDATE nodes
        SET sort_order = (
                SELECT new_order.new_sort_order
                FROM new_order
                WHERE new_order.id = nodes.id
            ),
            updated_at = ?5
        WHERE id IN (SELECT id FROM new_order)
        RETURNING id, sort_order, title
        ",
        )
        .map_err(|source| AppError::sqlite("prepare SQLite sibling sort order update", source))?;

    let rows = statement
        .query_map(
            params![
                parent_id,
                node_id,
                neighbor_node_id,
                direction_delta,
                updated_at
            ],
            |row| sibling_order_update_row_from_row(row, parent_id, updated_at),
        )
        .map_err(|source| AppError::sqlite("update SQLite sibling sort order", source))?;

    let update_rows =
        collect_sibling_order_update_rows(rows, "read recalculated SQLite sibling row")?;
    if !update_rows.iter().any(|row| row.update.node_id == node_id) {
        return Err(DomainError::NodeNotFound { node_id }.into());
    }
    if !update_rows
        .iter()
        .any(|row| row.update.node_id == neighbor_node_id)
    {
        return Err(DomainError::NodeNotFound {
            node_id: neighbor_node_id,
        }
        .into());
    }

    Ok(sorted_sibling_order_updates(update_rows))
}

pub(in crate::infra::sqlite) fn shift_sibling_sort_orders_after_removal(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    removed_sort_order: i64,
    excluded_node_id: Option<i64>,
    updated_at: &str,
) -> Result<Vec<NodeSiblingOrderUpdate>, AppError> {
    let mut statement = transaction
        .prepare(
            "
        UPDATE nodes
        SET sort_order = sort_order - 1, updated_at = ?1
        WHERE deleted_at IS NULL
            AND ((parent_id IS NULL AND ?2 IS NULL) OR parent_id = ?2)
            AND sort_order > ?3
            AND (?4 IS NULL OR id <> ?4)
        RETURNING id, sort_order, title
        ",
        )
        .map_err(|source| AppError::sqlite("prepare shifted SQLite sibling update", source))?;

    let rows = statement
        .query_map(
            params![updated_at, parent_id, removed_sort_order, excluded_node_id],
            |row| sibling_order_update_row_from_row(row, parent_id, updated_at),
        )
        .map_err(|source| AppError::sqlite("shift SQLite sibling sort order", source))?;

    let updates = collect_sibling_order_update_rows(rows, "read shifted SQLite sibling row")?;
    Ok(sorted_sibling_order_updates(updates))
}

pub(in crate::infra::sqlite) fn recalculate_sibling_sort_order(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    ordered_node_ids: &[i64],
    updated_at: &str,
) -> Result<(), AppError> {
    if ordered_node_ids.is_empty() {
        return Ok(());
    }

    transaction
        .execute(
            "
        CREATE TEMP TABLE IF NOT EXISTS sibling_sort_order_recalc (
            id INTEGER PRIMARY KEY,
            sort_order INTEGER NOT NULL
        )
        ",
            [],
        )
        .map_err(|source| {
            AppError::sqlite("create SQLite sibling sort order temp table", source)
        })?;
    transaction
        .execute("DELETE FROM sibling_sort_order_recalc", [])
        .map_err(|source| AppError::sqlite("clear SQLite sibling sort order temp table", source))?;

    {
        let mut statement = transaction
            .prepare(
                "
            INSERT OR IGNORE INTO sibling_sort_order_recalc (id, sort_order)
            VALUES (?1, ?2)
            ",
            )
            .map_err(|source| {
                AppError::sqlite("prepare SQLite sibling sort order temp insert", source)
            })?;

        for (sort_order, node_id) in ordered_node_ids.iter().enumerate() {
            let sort_order = i64::try_from(sort_order).map_err(|_| {
                AppError::internal_consistency(
                    "recalculate sibling sort order",
                    "too many sibling nodes to sort",
                )
            })?;

            statement
                .execute(params![node_id, sort_order])
                .map_err(|source| {
                    AppError::sqlite("insert SQLite sibling sort order temp row", source)
                })?;
        }
    }

    let existing_count: usize = transaction
        .query_row(
            "
        SELECT COUNT(*)
        FROM nodes
        WHERE id IN (SELECT id FROM sibling_sort_order_recalc)
            AND deleted_at IS NULL
            AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
        ",
            params![parent_id],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("count SQLite sibling sort order rows", source))?;
    if existing_count != ordered_node_ids.len() {
        for node_id in ordered_node_ids {
            let exists: bool = transaction
                .query_row(
                    "
                SELECT EXISTS (
                    SELECT 1
                    FROM nodes
                    WHERE id = ?1
                        AND deleted_at IS NULL
                        AND ((parent_id IS NULL AND ?2 IS NULL) OR parent_id = ?2)
                )
                ",
                    params![node_id, parent_id],
                    |row| row.get(0),
                )
                .map_err(|source| {
                    AppError::sqlite("check SQLite sibling sort order row", source)
                })?;
            if !exists {
                return Err(DomainError::NodeNotFound { node_id: *node_id }.into());
            }
        }

        return Err(AppError::internal_consistency(
            "recalculate sibling sort order",
            "sibling order contains duplicate nodes",
        ));
    }

    transaction
        .execute(
            "
        UPDATE nodes
        SET sort_order = (
                SELECT new_order.sort_order
                FROM sibling_sort_order_recalc AS new_order
                WHERE new_order.id = nodes.id
            ),
            updated_at = ?1
        WHERE id IN (SELECT id FROM sibling_sort_order_recalc)
            AND deleted_at IS NULL
            AND ((parent_id IS NULL AND ?2 IS NULL) OR parent_id = ?2)
        ",
            params![updated_at, parent_id],
        )
        .map_err(|source| AppError::sqlite("update SQLite sibling sort order", source))?;

    Ok(())
}

pub(in crate::infra::sqlite) fn sibling_order_updates(
    parent_id: Option<i64>,
    ordered_node_ids: &[i64],
    updated_at: &str,
) -> Result<Vec<NodeSiblingOrderUpdate>, AppError> {
    let mut updates = Vec::with_capacity(ordered_node_ids.len());
    for (sort_order, node_id) in ordered_node_ids.iter().enumerate() {
        let sort_order = i64::try_from(sort_order).map_err(|_| {
            AppError::internal_consistency(
                "build sibling sort order update",
                "too many sibling nodes to sort",
            )
        })?;
        updates.push(NodeSiblingOrderUpdate {
            node_id: *node_id,
            parent_id,
            sort_order,
            updated_at: updated_at.to_owned(),
        });
    }
    Ok(updates)
}

fn sibling_order_update_row_from_row(
    row: &Row<'_>,
    parent_id: Option<i64>,
    updated_at: &str,
) -> rusqlite::Result<SiblingOrderUpdateRow> {
    let node_id = row.get(0)?;
    let sort_order = row.get(1)?;
    let title = row.get(2)?;

    Ok(SiblingOrderUpdateRow {
        title,
        update: NodeSiblingOrderUpdate {
            node_id,
            parent_id,
            sort_order,
            updated_at: updated_at.to_owned(),
        },
    })
}

fn collect_sibling_order_update_rows<F>(
    rows: MappedRows<'_, F>,
    context: &'static str,
) -> Result<Vec<SiblingOrderUpdateRow>, AppError>
where
    F: FnMut(&Row<'_>) -> rusqlite::Result<SiblingOrderUpdateRow>,
{
    let mut updates = Vec::new();
    for row in rows {
        updates.push(row.map_err(|source| AppError::sqlite(context, source))?);
    }
    Ok(updates)
}

fn sorted_sibling_order_updates(
    mut updates: Vec<SiblingOrderUpdateRow>,
) -> Vec<NodeSiblingOrderUpdate> {
    updates.sort_by(|left, right| {
        left.update
            .sort_order
            .cmp(&right.update.sort_order)
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.update.node_id.cmp(&right.update.node_id))
    });

    updates.into_iter().map(|row| row.update).collect()
}

pub(in crate::infra::sqlite) fn next_child_sort_order(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
) -> Result<i64, AppError> {
    let max_sort_order: Option<i64> = transaction
        .query_row(
            "
        SELECT MAX(sort_order)
        FROM nodes
        WHERE deleted_at IS NULL
            AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
        ",
            params![parent_id],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("read max SQLite child sort order", source))?;

    let Some(max_sort_order) = max_sort_order else {
        return Ok(0);
    };

    max_sort_order.checked_add(1).ok_or_else(|| {
        AppError::internal_consistency(
            "calculate next SQLite child sort order",
            "SQLite child sort_order reached i64::MAX",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn create_ordering_connection() -> Result<Connection, AppError> {
        let connection = Connection::open_in_memory()
            .map_err(|source| AppError::sqlite("open in-memory SQLite database", source))?;
        connection
            .execute_batch(
                "
                CREATE TABLE nodes (
                    id INTEGER PRIMARY KEY,
                    parent_id INTEGER NULL,
                    title TEXT NOT NULL,
                    sort_order INTEGER NOT NULL,
                    updated_at TEXT NOT NULL,
                    deleted_at TEXT NULL
                );
                ",
            )
            .map_err(|source| AppError::sqlite("create test SQLite nodes table", source))?;
        Ok(connection)
    }

    #[test]
    fn next_child_sort_order_returns_app_error_when_max_sort_order_is_i64_max(
    ) -> Result<(), AppError> {
        let mut connection = create_ordering_connection()?;
        let parent_id = Some(10);
        insert_active_node(&connection, 2, parent_id, "Max", i64::MAX)?;

        let transaction = connection
            .transaction()
            .map_err(|source| AppError::sqlite("start test SQLite transaction", source))?;

        match next_child_sort_order(&transaction, parent_id) {
            Err(AppError::User { .. }) => Ok(()),
            Err(AppError::Sqlite { .. }) => Err(AppError::user(
                "next_child_sort_order returned a SQLite error instead of an app error",
            )),
            Err(error) => Err(error),
            Ok(sort_order) => Err(AppError::user(format!(
                "next_child_sort_order returned {sort_order} instead of an overflow error"
            ))),
        }
    }

    fn insert_active_node(
        connection: &Connection,
        id: i64,
        parent_id: Option<i64>,
        title: &str,
        sort_order: i64,
    ) -> Result<(), AppError> {
        connection
            .execute(
                "
                INSERT INTO nodes (id, parent_id, title, sort_order, updated_at, deleted_at)
                VALUES (?1, ?2, ?3, ?4, 'created', NULL)
                ",
                params![id, parent_id, title, sort_order],
            )
            .map_err(|source| AppError::sqlite("insert test SQLite node", source))?;
        Ok(())
    }

    fn active_node_order(
        connection: &Connection,
        parent_id: Option<i64>,
    ) -> Result<Vec<i64>, AppError> {
        let mut statement = connection
            .prepare(
                "
                SELECT id
                FROM nodes
                WHERE deleted_at IS NULL
                    AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
                ORDER BY sort_order, title, id
                ",
            )
            .map_err(|source| AppError::sqlite("prepare test SQLite node order query", source))?;
        let rows = statement
            .query_map(params![parent_id], |row| row.get(0))
            .map_err(|source| AppError::sqlite("query test SQLite node order", source))?;

        let mut node_ids = Vec::new();
        for row in rows {
            node_ids.push(
                row.map_err(|source| AppError::sqlite("read test SQLite node order row", source))?,
            );
        }
        Ok(node_ids)
    }

    fn active_node_sort_orders(
        connection: &Connection,
        parent_id: Option<i64>,
    ) -> Result<Vec<(i64, i64)>, AppError> {
        let mut statement = connection
            .prepare(
                "
                SELECT id, sort_order
                FROM nodes
                WHERE deleted_at IS NULL
                    AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
                ORDER BY sort_order, title, id
                ",
            )
            .map_err(|source| {
                AppError::sqlite("prepare test SQLite node sort order query", source)
            })?;
        let rows = statement
            .query_map(params![parent_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|source| AppError::sqlite("query test SQLite node sort order", source))?;

        let mut sort_orders = Vec::new();
        for row in rows {
            sort_orders.push(row.map_err(|source| {
                AppError::sqlite("read test SQLite node sort order row", source)
            })?);
        }
        Ok(sort_orders)
    }

    fn active_node_updated_ats(
        connection: &Connection,
        parent_id: Option<i64>,
    ) -> Result<Vec<(i64, String)>, AppError> {
        let mut statement = connection
            .prepare(
                "
                SELECT id, updated_at
                FROM nodes
                WHERE deleted_at IS NULL
                    AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
                ORDER BY id
                ",
            )
            .map_err(|source| {
                AppError::sqlite("prepare test SQLite node timestamp query", source)
            })?;
        let rows = statement
            .query_map(params![parent_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|source| AppError::sqlite("query test SQLite node timestamp", source))?;

        let mut timestamps = Vec::new();
        for row in rows {
            timestamps.push(row.map_err(|source| {
                AppError::sqlite("read test SQLite node timestamp row", source)
            })?);
        }
        Ok(timestamps)
    }

    #[test]
    fn shift_after_removal_returns_rows_in_sibling_order() -> Result<(), AppError> {
        let mut connection = create_ordering_connection()?;
        let parent_id = Some(10);
        insert_active_node(&connection, 3, parent_id, "C", 1)?;
        insert_active_node(&connection, 4, parent_id, "A", 1)?;
        insert_active_node(&connection, 5, parent_id, "B", 2)?;
        insert_active_node(&connection, 6, parent_id, "D", 3)?;

        let transaction = connection
            .transaction()
            .map_err(|source| AppError::sqlite("start test SQLite transaction", source))?;
        let updates = shift_sibling_sort_orders_after_removal(
            &transaction,
            parent_id,
            0,
            Some(6),
            "updated",
        )?;
        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit test SQLite transaction", source))?;

        let update_rows = updates
            .iter()
            .map(|update| {
                (
                    update.node_id,
                    update.parent_id,
                    update.sort_order,
                    update.updated_at.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            update_rows,
            vec![
                (4, parent_id, 0, "updated"),
                (3, parent_id, 0, "updated"),
                (5, parent_id, 1, "updated"),
            ]
        );
        assert_eq!(
            active_node_sort_orders(&connection, parent_id)?,
            vec![(4, 0), (3, 0), (5, 1), (6, 3)]
        );
        Ok(())
    }

    #[test]
    fn reorder_duplicate_sort_order_updates_bucket_and_shifted_later_siblings(
    ) -> Result<(), AppError> {
        let mut connection = create_ordering_connection()?;
        let parent_id = Some(10);
        insert_active_node(&connection, 1, parent_id, "Before", 0)?;
        insert_active_node(&connection, 2, parent_id, "A", 5)?;
        insert_active_node(&connection, 3, parent_id, "B", 5)?;
        insert_active_node(&connection, 4, parent_id, "C", 5)?;
        insert_active_node(&connection, 5, parent_id, "After", 6)?;

        let transaction = connection
            .transaction()
            .map_err(|source| AppError::sqlite("start test SQLite transaction", source))?;
        let updates = reorder_duplicate_sibling_sort_order(
            &transaction,
            parent_id,
            3,
            4,
            5,
            &SiblingMoveDirection::Down,
            "updated",
        )?;
        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit test SQLite transaction", source))?;

        let update_rows = updates
            .iter()
            .map(|update| {
                (
                    update.node_id,
                    update.parent_id,
                    update.sort_order,
                    update.updated_at.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            update_rows,
            vec![
                (4, parent_id, 6, "updated"),
                (3, parent_id, 7, "updated"),
                (5, parent_id, 8, "updated"),
            ]
        );
        assert_eq!(
            active_node_order(&connection, parent_id)?,
            vec![1, 2, 4, 3, 5]
        );
        assert_eq!(
            active_node_sort_orders(&connection, parent_id)?,
            vec![(1, 0), (2, 5), (4, 6), (3, 7), (5, 8)]
        );
        assert_eq!(
            active_node_updated_ats(&connection, parent_id)?,
            vec![
                (1, "created".to_owned()),
                (2, "created".to_owned()),
                (3, "updated".to_owned()),
                (4, "updated".to_owned()),
                (5, "updated".to_owned()),
            ]
        );
        Ok(())
    }

    #[test]
    fn reorder_duplicate_sort_order_uses_existing_gap_without_shifting_later_siblings(
    ) -> Result<(), AppError> {
        let mut connection = create_ordering_connection()?;
        let parent_id = Some(10);
        insert_active_node(&connection, 2, parent_id, "A", 5)?;
        insert_active_node(&connection, 3, parent_id, "B", 5)?;
        insert_active_node(&connection, 4, parent_id, "After", 10)?;

        let transaction = connection
            .transaction()
            .map_err(|source| AppError::sqlite("start test SQLite transaction", source))?;
        let updates = reorder_duplicate_sibling_sort_order(
            &transaction,
            parent_id,
            2,
            3,
            5,
            &SiblingMoveDirection::Down,
            "updated",
        )?;
        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit test SQLite transaction", source))?;

        let update_rows = updates
            .iter()
            .map(|update| {
                (
                    update.node_id,
                    update.parent_id,
                    update.sort_order,
                    update.updated_at.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            update_rows,
            vec![(3, parent_id, 5, "updated"), (2, parent_id, 6, "updated")]
        );
        assert_eq!(active_node_order(&connection, parent_id)?, vec![3, 2, 4]);
        assert_eq!(
            active_node_sort_orders(&connection, parent_id)?,
            vec![(3, 5), (2, 6), (4, 10)]
        );
        assert_eq!(
            active_node_updated_ats(&connection, parent_id)?,
            vec![
                (2, "updated".to_owned()),
                (3, "updated".to_owned()),
                (4, "created".to_owned()),
            ]
        );
        Ok(())
    }

    #[test]
    fn move_current_parent_end_shifts_only_later_unique_sort_orders() -> Result<(), AppError> {
        let mut connection = create_ordering_connection()?;
        let parent_id = Some(10);
        insert_active_node(&connection, 2, parent_id, "A", 0)?;
        insert_active_node(&connection, 3, parent_id, "B", 1)?;
        insert_active_node(&connection, 4, parent_id, "C", 2)?;

        let transaction = connection
            .transaction()
            .map_err(|source| AppError::sqlite("start test SQLite transaction", source))?;
        let updates =
            move_node_to_current_parent_end(&transaction, 2, parent_id, "A", 0, "updated")?;
        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit test SQLite transaction", source))?;

        let update_rows = updates
            .iter()
            .map(|update| {
                (
                    update.node_id,
                    update.parent_id,
                    update.sort_order,
                    update.updated_at.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            update_rows,
            vec![
                (3, parent_id, 0, "updated"),
                (4, parent_id, 1, "updated"),
                (2, parent_id, 2, "updated"),
            ]
        );
        assert_eq!(active_node_order(&connection, parent_id)?, vec![3, 4, 2]);
        assert_eq!(
            active_node_sort_orders(&connection, parent_id)?,
            vec![(3, 0), (4, 1), (2, 2)]
        );
        Ok(())
    }

    #[test]
    fn move_current_parent_end_reorders_duplicate_max_sort_order() -> Result<(), AppError> {
        let mut connection = create_ordering_connection()?;
        let parent_id = Some(10);
        insert_active_node(&connection, 2, parent_id, "A", 5)?;
        insert_active_node(&connection, 3, parent_id, "B", 5)?;
        insert_active_node(&connection, 4, parent_id, "C", 5)?;

        let transaction = connection
            .transaction()
            .map_err(|source| AppError::sqlite("start test SQLite transaction", source))?;
        let updates =
            move_node_to_current_parent_end(&transaction, 2, parent_id, "A", 5, "updated")?;
        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit test SQLite transaction", source))?;

        let update_rows = updates
            .iter()
            .map(|update| {
                (
                    update.node_id,
                    update.parent_id,
                    update.sort_order,
                    update.updated_at.as_str(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            update_rows,
            vec![
                (3, parent_id, 0, "updated"),
                (4, parent_id, 1, "updated"),
                (2, parent_id, 2, "updated"),
            ]
        );
        assert_eq!(active_node_order(&connection, parent_id)?, vec![3, 4, 2]);
        assert_eq!(
            active_node_sort_orders(&connection, parent_id)?,
            vec![(3, 0), (4, 1), (2, 2)]
        );
        Ok(())
    }

    #[test]
    fn move_current_parent_end_noops_when_duplicate_max_is_last() -> Result<(), AppError> {
        let mut connection = create_ordering_connection()?;
        let parent_id = Some(10);
        insert_active_node(&connection, 2, parent_id, "A", 5)?;
        insert_active_node(&connection, 3, parent_id, "B", 5)?;
        insert_active_node(&connection, 4, parent_id, "C", 5)?;

        let transaction = connection
            .transaction()
            .map_err(|source| AppError::sqlite("start test SQLite transaction", source))?;
        let updates =
            move_node_to_current_parent_end(&transaction, 4, parent_id, "C", 5, "updated")?;
        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit test SQLite transaction", source))?;

        assert!(updates.is_empty());
        assert_eq!(active_node_order(&connection, parent_id)?, vec![2, 3, 4]);
        assert_eq!(
            active_node_sort_orders(&connection, parent_id)?,
            vec![(2, 5), (3, 5), (4, 5)]
        );
        Ok(())
    }
}
