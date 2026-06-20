use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior};

use crate::domain::Node;
use crate::error::AppError;

use super::node::{current_timestamp, insert_node};
use super::{schema, SqliteDocumentRepository};

pub(super) const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

impl SqliteDocumentRepository {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        let connection = Connection::open(path)
            .map_err(|source| AppError::database_open(path.to_path_buf(), source))?;
        connection
            .busy_timeout(SQLITE_BUSY_TIMEOUT)
            .map_err(|source| AppError::sqlite("set SQLite busy timeout", source))?;
        connection
            .execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|source| AppError::sqlite("enable SQLite foreign keys", source))?;
        Ok(Self { connection })
    }

    pub fn migrate(&mut self) -> Result<(), AppError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| AppError::sqlite("start SQLite migration", source))?;

        schema::migrate(&transaction)?;

        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit SQLite migration", source))
    }

    pub fn ensure_initial_content(&mut self) -> Result<(), AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| AppError::sqlite("start initial content transaction", source))?;
        let existing_nodes: i64 = transaction
            .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))
            .map_err(|source| AppError::sqlite("count SQLite nodes", source))?;

        if existing_nodes > 0 {
            return transaction
                .commit()
                .map_err(|source| AppError::sqlite("commit initial content check", source));
        }

        let now = current_timestamp(&transaction)?;
        let root = Node::root_document(now.clone(), now.clone());
        let document = Node::default_document(now.clone(), now);

        root.validate()?;
        document.validate()?;

        insert_node(&transaction, &root)?;
        insert_node(&transaction, &document)?;

        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit initial content", source))
    }
}
