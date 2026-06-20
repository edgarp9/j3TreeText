use rusqlite::{params, Connection};

use crate::domain::UiSettings;
use crate::error::AppError;

use super::SqliteDocumentRepository;

impl SqliteDocumentRepository {
    pub fn load_ui_settings(&self) -> Result<UiSettings, AppError> {
        load(&self.connection)
    }

    pub fn save_ui_settings(&mut self, settings: &UiSettings) -> Result<(), AppError> {
        save(&mut self.connection, settings)
    }

    pub fn save_changed_ui_settings(
        &mut self,
        previous: &UiSettings,
        settings: &UiSettings,
    ) -> Result<(), AppError> {
        save_entries(&mut self.connection, settings.changed_entries(previous))
    }
}

pub(super) fn load(connection: &Connection) -> Result<UiSettings, AppError> {
    let mut statement = connection
        .prepare(
            "
            SELECT key, value
            FROM settings
            ORDER BY key
            ",
        )
        .map_err(|source| AppError::sqlite("prepare settings query", source))?;

    let rows = statement
        .query_map([], |row| {
            let key: String = row.get(0)?;
            let value: String = row.get(1)?;
            Ok((key, value))
        })
        .map_err(|source| AppError::sqlite("query SQLite settings", source))?;

    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.map_err(|source| AppError::sqlite("read SQLite setting row", source))?);
    }

    Ok(UiSettings::from_entries(
        entries
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str())),
    ))
}

pub(super) fn save(connection: &mut Connection, settings: &UiSettings) -> Result<(), AppError> {
    save_entries(connection, settings.entries())
}

fn save_entries(
    connection: &mut Connection,
    entries: Vec<(&'static str, String)>,
) -> Result<(), AppError> {
    if entries.is_empty() {
        return Ok(());
    }

    let transaction = connection
        .transaction()
        .map_err(|source| AppError::sqlite("start settings transaction", source))?;

    for (key, value) in entries {
        transaction
            .execute(
                "
                INSERT INTO settings (key, value)
                VALUES (?1, ?2)
                ON CONFLICT(key) DO UPDATE SET value = excluded.value
                ",
                params![key, value],
            )
            .map_err(|source| AppError::sqlite("save SQLite setting", source))?;
    }

    transaction
        .commit()
        .map_err(|source| AppError::sqlite("commit settings transaction", source))
}
