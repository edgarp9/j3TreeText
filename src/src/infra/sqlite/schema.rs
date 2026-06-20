use rusqlite::{params, Transaction};

use crate::error::AppError;

use super::node::current_timestamp;

pub(super) const SCHEMA_VERSION: i64 = 6;

pub(super) fn migrate(transaction: &Transaction<'_>) -> Result<(), AppError> {
    transaction
        .execute_batch(
            "
            CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS nodes (
                id INTEGER PRIMARY KEY,
                parent_id INTEGER NULL,
                title TEXT NOT NULL CHECK (length(trim(title)) > 0),
                sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
                content TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                deleted_at TEXT NULL,
                FOREIGN KEY(parent_id) REFERENCES nodes(id)
            );
            ",
        )
        .map_err(|source| AppError::sqlite("create SQLite base schema", source))?;

    let applied_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = ?1",
            params![SCHEMA_VERSION],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("read SQLite schema version", source))?;
    let should_apply_current_migration = applied_count == 0;

    ensure_current_node_schema(transaction, should_apply_current_migration)?;
    let search_index_schema_refreshed = ensure_search_index_schema(transaction)?;

    if should_apply_current_migration {
        transaction
            .execute_batch(
                "
                DROP INDEX IF EXISTS idx_nodes_parent_sort;
                DROP INDEX IF EXISTS idx_nodes_active_root_title;
                DROP INDEX IF EXISTS idx_nodes_active_sibling_title;

                CREATE INDEX IF NOT EXISTS idx_nodes_parent_sort
                    ON nodes(parent_id, sort_order, id);

                CREATE UNIQUE INDEX IF NOT EXISTS idx_nodes_active_root_title
                    ON nodes(title)
                    WHERE deleted_at IS NULL AND parent_id IS NULL;

                CREATE UNIQUE INDEX IF NOT EXISTS idx_nodes_active_sibling_title
                    ON nodes(parent_id, title)
                    WHERE deleted_at IS NULL AND parent_id IS NOT NULL;

                CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                ",
            )
            .map_err(|source| AppError::sqlite("create SQLite indexes and settings", source))?;

        rebuild_search_index(transaction)?;

        transaction
            .execute(
                "
                INSERT INTO schema_migrations (version, name, applied_at)
                VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                ",
                params![SCHEMA_VERSION, "node_search_validated_content"],
            )
            .map_err(|source| AppError::sqlite("record SQLite schema migration", source))?;
    }

    if !should_apply_current_migration && search_index_schema_refreshed {
        rebuild_search_index(transaction)?;
    }

    ensure_active_nodes_display_order_index(transaction)?;
    ensure_deleted_nodes_display_order_index(transaction)?;

    Ok(())
}

fn ensure_current_node_schema(
    transaction: &Transaction<'_>,
    backfill_existing_nodes: bool,
) -> Result<(), AppError> {
    let mut columns = table_column_names(transaction, "nodes")?;
    let mut added_missing_node_column = false;

    if columns
        .iter()
        .any(|column| column.eq_ignore_ascii_case("kind"))
    {
        return Err(AppError::user(
        "이 데이터베이스는 폴더/문서 구분을 사용하는 이전 형식입니다. 자동 변환은 지원하지 않습니다. 새 데이터베이스를 사용하거나 별도 변환 도구로 이전하세요.",
    ));
    }

    added_missing_node_column |= ensure_node_column(
        transaction,
        &mut columns,
        "parent_id",
        "ALTER TABLE nodes ADD COLUMN parent_id INTEGER NULL",
    )?;
    added_missing_node_column |= ensure_node_column(
        transaction,
        &mut columns,
        "title",
        "ALTER TABLE nodes ADD COLUMN title TEXT NOT NULL DEFAULT 'Untitled'",
    )?;
    added_missing_node_column |= ensure_node_column(
        transaction,
        &mut columns,
        "sort_order",
        "ALTER TABLE nodes ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0",
    )?;
    added_missing_node_column |= ensure_node_column(
        transaction,
        &mut columns,
        "content",
        "ALTER TABLE nodes ADD COLUMN content TEXT NOT NULL DEFAULT ''",
    )?;
    added_missing_node_column |= ensure_node_column(
        transaction,
        &mut columns,
        "created_at",
        "ALTER TABLE nodes ADD COLUMN created_at TEXT NOT NULL DEFAULT ''",
    )?;
    added_missing_node_column |= ensure_node_column(
        transaction,
        &mut columns,
        "updated_at",
        "ALTER TABLE nodes ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''",
    )?;
    added_missing_node_column |= ensure_node_column(
        transaction,
        &mut columns,
        "deleted_at",
        "ALTER TABLE nodes ADD COLUMN deleted_at TEXT NULL",
    )?;

    if backfill_existing_nodes || added_missing_node_column {
        backfill_missing_node_defaults(transaction)?;
    }

    Ok(())
}

fn ensure_search_index_schema(transaction: &Transaction<'_>) -> Result<bool, AppError> {
    if !search_index_schema_needs_refresh(transaction)? {
        return Ok(false);
    }

    transaction
        .execute_batch(
            "
        CREATE VIRTUAL TABLE IF NOT EXISTS node_search_fts
        USING fts5(title, content, tokenize = 'trigram');

        CREATE TABLE IF NOT EXISTS node_search_validated_content (
            node_id INTEGER PRIMARY KEY,
            updated_at TEXT NOT NULL
        );

        DROP TRIGGER IF EXISTS node_search_fts_insert;
        DROP TRIGGER IF EXISTS node_search_fts_update;
        DROP TRIGGER IF EXISTS node_search_fts_content_or_deleted_update;
        DROP TRIGGER IF EXISTS node_search_fts_title_update;
        DROP TRIGGER IF EXISTS node_search_fts_delete;

        CREATE TRIGGER node_search_fts_insert
        AFTER INSERT ON nodes
        BEGIN
            INSERT INTO node_search_fts(rowid, title, content)
            SELECT new.id, new.title, new.content
            WHERE CASE
                WHEN new.deleted_at IS NOT NULL THEN 0
                WHEN instr(new.title, char(0)) != 0 THEN 0
                WHEN EXISTS (
                    SELECT 1
                    FROM node_search_validated_content
                    WHERE node_id = new.id
                        AND updated_at = new.updated_at
                ) THEN 1
                ELSE instr(new.content, char(0)) = 0
            END;
            DELETE FROM node_search_validated_content
            WHERE node_id = new.id;
        END;

        CREATE TRIGGER node_search_fts_update
        AFTER UPDATE OF content, deleted_at ON nodes
        BEGIN
            DELETE FROM node_search_fts WHERE rowid = old.id;
            INSERT INTO node_search_fts(rowid, title, content)
            SELECT new.id, new.title, new.content
            WHERE CASE
                WHEN new.deleted_at IS NOT NULL THEN 0
                WHEN instr(new.title, char(0)) != 0 THEN 0
                WHEN EXISTS (
                    SELECT 1
                    FROM node_search_validated_content
                    WHERE node_id = new.id
                        AND updated_at = new.updated_at
                ) THEN 1
                ELSE instr(new.content, char(0)) = 0
            END;
            DELETE FROM node_search_validated_content
            WHERE node_id = new.id;
        END;

        CREATE TRIGGER node_search_fts_title_update
        AFTER UPDATE OF title ON nodes
        WHEN old.content IS new.content AND old.deleted_at IS new.deleted_at
        BEGIN
            DELETE FROM node_search_fts
            WHERE rowid = old.id
                AND (new.deleted_at IS NOT NULL OR instr(new.title, char(0)) != 0);

            UPDATE node_search_fts
            SET title = new.title
            WHERE rowid = new.id
                AND new.deleted_at IS NULL
                AND instr(new.title, char(0)) = 0;

            INSERT INTO node_search_fts(rowid, title, content)
            SELECT new.id, new.title, new.content
            WHERE new.deleted_at IS NULL
                AND instr(new.title, char(0)) = 0
                AND NOT EXISTS (
                    SELECT 1
                    FROM node_search_fts
                    WHERE rowid = new.id
                )
                AND CASE
                    WHEN EXISTS (
                        SELECT 1
                        FROM node_search_validated_content
                        WHERE node_id = new.id
                            AND updated_at = new.updated_at
                    ) THEN 1
                    ELSE instr(new.content, char(0)) = 0
                END;
            DELETE FROM node_search_validated_content
            WHERE node_id = new.id;
        END;

        CREATE TRIGGER node_search_fts_delete
        AFTER DELETE ON nodes
        BEGIN
            DELETE FROM node_search_fts WHERE rowid = old.id;
            DELETE FROM node_search_validated_content
            WHERE node_id = old.id;
        END;
        ",
        )
        .map_err(|source| AppError::sqlite("create SQLite search index", source))?;

    Ok(true)
}

fn ensure_active_nodes_display_order_index(transaction: &Transaction<'_>) -> Result<(), AppError> {
    transaction
        .execute_batch(
            "
        CREATE INDEX IF NOT EXISTS idx_nodes_active_display_order
        ON nodes(
            parent_id IS NOT NULL,
            parent_id,
            sort_order,
            title,
            id,
            created_at,
            updated_at,
            deleted_at
        )
        WHERE deleted_at IS NULL;
        ",
        )
        .map_err(|source| AppError::sqlite("create SQLite active nodes display index", source))
}

fn ensure_deleted_nodes_display_order_index(transaction: &Transaction<'_>) -> Result<(), AppError> {
    transaction
        .execute_batch(
            "
        CREATE INDEX IF NOT EXISTS idx_nodes_deleted_display_order
        ON nodes(
            deleted_at DESC,
            parent_id IS NOT NULL,
            parent_id,
            sort_order,
            title,
            id,
            created_at,
            updated_at
        )
        WHERE deleted_at IS NOT NULL;
        ",
        )
        .map_err(|source| AppError::sqlite("create SQLite deleted nodes display index", source))
}

fn search_index_schema_needs_refresh(transaction: &Transaction<'_>) -> Result<bool, AppError> {
    if !schema_object_exists(transaction, "table", "node_search_fts")?
        || !schema_object_exists(transaction, "table", "node_search_validated_content")?
        || !schema_object_exists(transaction, "trigger", "node_search_fts_insert")?
        || !schema_object_exists(transaction, "trigger", "node_search_fts_delete")?
        || !schema_object_exists(transaction, "trigger", "node_search_fts_title_update")?
        || !schema_object_exists(transaction, "trigger", "node_search_fts_update")?
        || schema_object_exists(
            transaction,
            "trigger",
            "node_search_fts_content_or_deleted_update",
        )?
    {
        return Ok(true);
    }

    let title_trigger = schema_object_sql(transaction, "trigger", "node_search_fts_title_update")?;
    let content_trigger = schema_object_sql(transaction, "trigger", "node_search_fts_update")?;

    Ok(!title_trigger
        .contains("WHEN old.content IS new.content AND old.deleted_at IS new.deleted_at")
        || !title_trigger.contains("UPDATE node_search_fts")
        || !content_trigger.contains("AFTER UPDATE OF content, deleted_at ON nodes"))
}

fn schema_object_exists(
    transaction: &Transaction<'_>,
    object_type: &str,
    name: &str,
) -> Result<bool, AppError> {
    let count: i64 = transaction
        .query_row(
            "
            SELECT COUNT(*)
            FROM sqlite_schema
            WHERE type = ?1 AND name = ?2
            ",
            params![object_type, name],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("read SQLite schema object", source))?;

    Ok(count != 0)
}

fn schema_object_sql(
    transaction: &Transaction<'_>,
    object_type: &str,
    name: &str,
) -> Result<String, AppError> {
    transaction
        .query_row(
            "
            SELECT sql
            FROM sqlite_schema
            WHERE type = ?1 AND name = ?2
            ",
            params![object_type, name],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("read SQLite schema SQL", source))
}

fn rebuild_search_index(transaction: &Transaction<'_>) -> Result<(), AppError> {
    transaction
        .execute_batch(
            "
        DELETE FROM node_search_fts;

        INSERT INTO node_search_fts(rowid, title, content)
        SELECT id, title, content
        FROM nodes
        WHERE deleted_at IS NULL
            AND instr(title, char(0)) = 0
            AND instr(content, char(0)) = 0;
        ",
        )
        .map_err(|source| AppError::sqlite("rebuild SQLite search index", source))
}

fn table_column_names(
    transaction: &Transaction<'_>,
    table_name: &str,
) -> Result<Vec<String>, AppError> {
    let mut statement = transaction
        .prepare(&format!("PRAGMA table_info({table_name})"))
        .map_err(|source| AppError::sqlite("prepare SQLite table info query", source))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|source| AppError::sqlite("query SQLite table columns", source))?;

    let mut columns = Vec::new();
    for row in rows {
        columns.push(row.map_err(|source| AppError::sqlite("read SQLite table column", source))?);
    }

    Ok(columns)
}

fn ensure_node_column(
    transaction: &Transaction<'_>,
    columns: &mut Vec<String>,
    column_name: &str,
    add_column_sql: &str,
) -> Result<bool, AppError> {
    if columns
        .iter()
        .any(|column| column.eq_ignore_ascii_case(column_name))
    {
        return Ok(false);
    }

    transaction
        .execute_batch(add_column_sql)
        .map_err(|source| AppError::sqlite("add missing SQLite node column", source))?;
    columns.push(column_name.to_owned());
    Ok(true)
}

fn backfill_missing_node_defaults(transaction: &Transaction<'_>) -> Result<(), AppError> {
    let now = current_timestamp(transaction)?;

    transaction
        .execute(
            "
        UPDATE nodes
        SET created_at = ?1
        WHERE created_at IS NULL OR length(trim(created_at)) = 0
        ",
            params![&now],
        )
        .map_err(|source| AppError::sqlite("backfill SQLite node created_at", source))?;
    transaction
        .execute(
            "
        UPDATE nodes
        SET updated_at = ?1
        WHERE updated_at IS NULL OR length(trim(updated_at)) = 0
        ",
            params![&now],
        )
        .map_err(|source| AppError::sqlite("backfill SQLite node updated_at", source))?;
    transaction
        .execute(
            "
        UPDATE nodes
        SET sort_order = 0
        WHERE sort_order IS NULL OR sort_order < 0
        ",
            [],
        )
        .map_err(|source| AppError::sqlite("backfill SQLite node sort_order", source))?;
    transaction
        .execute(
            "
        UPDATE nodes
        SET content = ''
        WHERE content IS NULL
        ",
            [],
        )
        .map_err(|source| AppError::sqlite("backfill SQLite node content", source))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::{params, Connection};

    use super::*;

    fn create_current_nodes_table(connection: &Connection) {
        connection
            .execute_batch(
                "
                CREATE TABLE nodes (
                    id INTEGER PRIMARY KEY,
                    parent_id INTEGER NULL,
                    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
                    sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
                    content TEXT NOT NULL DEFAULT '',
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    deleted_at TEXT NULL,
                    FOREIGN KEY(parent_id) REFERENCES nodes(id)
                );
                ",
            )
            .expect("create current nodes table");
    }

    fn insert_node_with_timestamps(connection: &Connection, created_at: &str, updated_at: &str) {
        connection
            .execute(
                "
                INSERT INTO nodes (
                    id,
                    parent_id,
                    title,
                    sort_order,
                    content,
                    created_at,
                    updated_at,
                    deleted_at
                )
                VALUES (1, NULL, 'Root', 0, '', ?1, ?2, NULL)
                ",
                params![created_at, updated_at],
            )
            .expect("insert node");
    }

    fn read_node_timestamps(connection: &Connection) -> (String, String) {
        connection
            .query_row(
                "SELECT created_at, updated_at FROM nodes WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("read node timestamps")
    }

    #[test]
    fn ensure_current_node_schema_skips_backfill_when_not_needed() {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        create_current_nodes_table(&connection);
        insert_node_with_timestamps(&connection, "", "");

        let transaction = connection.transaction().expect("start transaction");
        ensure_current_node_schema(&transaction, false).expect("ensure current node schema");
        transaction.commit().expect("commit transaction");

        assert_eq!(
            read_node_timestamps(&connection),
            (String::new(), String::new())
        );
    }

    #[test]
    fn ensure_current_node_schema_backfills_when_current_migration_is_pending() {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        create_current_nodes_table(&connection);
        insert_node_with_timestamps(&connection, "", "");

        let transaction = connection.transaction().expect("start transaction");
        ensure_current_node_schema(&transaction, true).expect("ensure current node schema");
        transaction.commit().expect("commit transaction");

        let (created_at, updated_at) = read_node_timestamps(&connection);
        assert!(!created_at.is_empty());
        assert!(!updated_at.is_empty());
    }

    #[test]
    fn title_only_update_keeps_content_search_match() {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        create_current_nodes_table(&connection);

        let transaction = connection.transaction().expect("start transaction");
        ensure_search_index_schema(&transaction).expect("create search index schema");
        transaction.commit().expect("commit transaction");

        connection
            .execute(
                "
                INSERT INTO nodes (
                    id,
                    parent_id,
                    title,
                    sort_order,
                    content,
                    created_at,
                    updated_at,
                    deleted_at
                )
                VALUES (1, NULL, 'Alpha title', 0, 'body needle text', ?1, ?1, NULL)
                ",
                params!["2026-01-01T00:00:00.000Z"],
            )
            .expect("insert indexed node");

        connection
            .execute(
                "
                UPDATE nodes
                SET title = 'Beta title',
                    updated_at = '2026-01-01T00:00:01.000Z'
                WHERE id = 1
                ",
                [],
            )
            .expect("rename node");

        assert_eq!(search_match_count(&connection, "Beta"), 1);
        assert_eq!(search_match_count(&connection, "Alpha"), 0);
        assert_eq!(search_match_count(&connection, "needle"), 1);
    }

    #[test]
    fn search_triggers_split_title_updates_from_content_updates() {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        create_current_nodes_table(&connection);

        let transaction = connection.transaction().expect("start transaction");
        ensure_search_index_schema(&transaction).expect("create search index schema");
        transaction.commit().expect("commit transaction");

        let title_trigger = read_trigger_sql(&connection, "node_search_fts_title_update");
        let content_trigger = read_trigger_sql(&connection, "node_search_fts_update");

        assert!(title_trigger.contains("AFTER UPDATE OF title ON nodes"));
        assert!(title_trigger.contains("UPDATE node_search_fts"));
        assert!(content_trigger.contains("AFTER UPDATE OF content, deleted_at ON nodes"));
        assert!(content_trigger.contains("SELECT new.id, new.title, new.content"));
    }

    #[test]
    fn migrate_rebuilds_search_index_when_search_schema_is_recreated() {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        create_current_nodes_table(&connection);
        connection
            .execute_batch(
                "
                CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                );
                ",
            )
            .expect("create schema migrations table");
        connection
            .execute(
                "
                INSERT INTO schema_migrations (version, name, applied_at)
                VALUES (?1, 'node_search_validated_content', '2026-05-21T00:00:00.000Z')
                ",
                params![SCHEMA_VERSION],
            )
            .expect("record current schema version");
        connection
            .execute(
                "
                INSERT INTO nodes (
                    id,
                    parent_id,
                    title,
                    sort_order,
                    content,
                    created_at,
                    updated_at,
                    deleted_at
                )
                VALUES (
                    1,
                    NULL,
                    'Recovered needle',
                    0,
                    'body content for recovery search',
                    '2026-05-21T00:00:00.000Z',
                    '2026-05-21T00:00:00.000Z',
                    NULL
                )
                ",
                [],
            )
            .expect("insert existing node");

        let transaction = connection.transaction().expect("start transaction");
        migrate(&transaction).expect("migrate current database");
        transaction.commit().expect("commit transaction");

        assert_eq!(search_match_count(&connection, "needle"), 1);
        assert_eq!(search_match_count(&connection, "recovery"), 1);
    }

    #[test]
    fn active_nodes_metadata_query_uses_display_order_index() {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        create_current_nodes_table(&connection);

        let transaction = connection.transaction().expect("start transaction");
        ensure_active_nodes_display_order_index(&transaction)
            .expect("create active nodes display index");
        transaction.commit().expect("commit transaction");

        let plan = active_nodes_metadata_query_plan(&connection);

        assert!(
            plan.iter()
                .any(|detail| detail.contains("idx_nodes_active_display_order")),
            "query plan did not use active node display index: {plan:?}"
        );
        assert!(
            plan.iter()
                .all(|detail| !detail.contains("USE TEMP B-TREE")),
            "query plan still needs a temporary sort: {plan:?}"
        );
    }

    #[test]
    fn deleted_nodes_order_query_uses_display_order_index() {
        let mut connection = Connection::open_in_memory().expect("open in-memory database");
        create_current_nodes_table(&connection);

        let transaction = connection.transaction().expect("start transaction");
        ensure_deleted_nodes_display_order_index(&transaction)
            .expect("create deleted nodes display index");
        transaction.commit().expect("commit transaction");

        let plan = deleted_nodes_order_query_plan(&connection);

        assert!(
            plan.iter()
                .any(|detail| detail.contains("idx_nodes_deleted_display_order")),
            "query plan did not use deleted node display index: {plan:?}"
        );
        assert!(
            plan.iter()
                .all(|detail| !detail.contains("USE TEMP B-TREE")),
            "query plan still needs a temporary sort: {plan:?}"
        );
    }

    fn search_match_count(connection: &Connection, query: &str) -> i64 {
        connection
            .query_row(
                "SELECT COUNT(*) FROM node_search_fts WHERE node_search_fts MATCH ?1",
                params![query],
                |row| row.get(0),
            )
            .expect("read search match count")
    }

    fn read_trigger_sql(connection: &Connection, name: &str) -> String {
        connection
            .query_row(
                "
                SELECT sql
                FROM sqlite_schema
                WHERE type = 'trigger' AND name = ?1
                ",
                params![name],
                |row| row.get(0),
            )
            .expect("read trigger SQL")
    }

    fn active_nodes_metadata_query_plan(connection: &Connection) -> Vec<String> {
        let mut statement = connection
            .prepare(
                "
                EXPLAIN QUERY PLAN
                SELECT id, parent_id, title, sort_order, '' AS content, created_at, updated_at, deleted_at
                FROM nodes
                WHERE deleted_at IS NULL
                ORDER BY parent_id IS NOT NULL, parent_id, sort_order, title, id
                ",
            )
            .expect("prepare active nodes metadata query plan");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(3))
            .expect("query active nodes metadata query plan");

        let mut plan = Vec::new();
        for row in rows {
            plan.push(row.expect("read active nodes metadata query plan detail"));
        }
        plan
    }

    fn deleted_nodes_order_query_plan(connection: &Connection) -> Vec<String> {
        let mut statement = connection
            .prepare(
                "
                EXPLAIN QUERY PLAN
                SELECT id, parent_id, title, sort_order, '' AS content, created_at, updated_at, deleted_at
                FROM nodes
                WHERE deleted_at IS NOT NULL
                ORDER BY deleted_at DESC, parent_id IS NOT NULL, parent_id, sort_order, title, id
                ",
            )
            .expect("prepare deleted nodes query plan");
        let rows = statement
            .query_map([], |row| row.get::<_, String>(3))
            .expect("query deleted nodes query plan");

        let mut plan = Vec::new();
        for row in rows {
            plan.push(row.expect("read deleted nodes query plan detail"));
        }
        plan
    }
}
