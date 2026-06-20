use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Barrier,
};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::params;

use super::node::{current_timestamp, insert_node};
use super::repository::SQLITE_BUSY_TIMEOUT;
use super::{database_path_for_executable, schema, SqliteDocumentRepository};
use crate::domain::{
    AppearanceSettings, AppearanceTheme, AutoSaveSettings, Document, DomainError,
    EditorFontSettings, EditorSettings, Node, SelectionSettings, SiblingMoveDirection,
    SplitterSettings, TextEncoding, TextEncodingSettings, UiLanguage, UiSettings, WindowSettings,
    DEFAULT_DOCUMENT_ID, DEFAULT_EDITOR_FONT_SIZE_PT, DEFAULT_SPLITTER_LEFT_WIDTH,
    DEFAULT_WINDOW_WIDTH, ROOT_NODE_ID, SETTING_SELECTION_NODE_ID, SETTING_SPLITTER_LEFT_WIDTH,
};
use crate::error::AppError;

#[test]
fn database_path_replaces_executable_extension() -> Result<(), Box<dyn Error>> {
    let exe_path = Path::new("bin").join("j3TreeText.exe");
    let db_path = database_path_for_executable(&exe_path)?;

    assert_eq!(db_path, Path::new("bin").join("j3TreeText.db"));
    Ok(())
}

#[test]
fn database_path_adds_db_extension_when_executable_has_no_extension() -> Result<(), Box<dyn Error>>
{
    let exe_path = Path::new("bin").join("j3TreeText");
    let db_path = database_path_for_executable(&exe_path)?;

    assert_eq!(db_path, Path::new("bin").join("j3TreeText.db"));
    Ok(())
}

#[test]
fn opening_missing_database_creates_file_without_touching_user_database(
) -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let _repository = SqliteDocumentRepository::open(&db_path)?;
        assert!(db_path.exists());
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn opening_database_sets_busy_timeout() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let repository = SqliteDocumentRepository::open(&db_path)?;
        let busy_timeout_ms: i64 =
            repository
                .connection
                .query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
        let configured_timeout_ms = i64::try_from(SQLITE_BUSY_TIMEOUT.as_millis())?;

        assert_eq!(busy_timeout_ms, configured_timeout_ms);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn migration_creates_initial_document() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    if db_path.exists() {
        fs::remove_file(&db_path)?;
    }

    {
        let mut repository = SqliteDocumentRepository::open(&db_path)?;
        repository.migrate()?;
        repository.ensure_initial_content()?;
        let document = repository.load_document()?;

        assert!(db_path.exists());
        assert_eq!(document.node_count(), 2);

        let root = find_node(&document, ROOT_NODE_ID)?;
        assert_eq!(root.parent_id, None);
        assert_eq!(root.title, "Root");
        assert_eq!(root.sort_order, 0);
        assert_eq!(root.content, "");

        let default_document = find_node(&document, DEFAULT_DOCUMENT_ID)?;
        assert_eq!(default_document.parent_id, Some(ROOT_NODE_ID));
        assert_eq!(default_document.title, "Untitled");
        assert_eq!(default_document.sort_order, 0);
        assert_eq!(default_document.content, "");
    }

    if db_path.exists() {
        fs::remove_file(&db_path)?;
    }
    Ok(())
}

#[test]
fn loading_document_metadata_omits_content_until_requested() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let document =
            repository.create_document_with_content(ROOT_NODE_ID, "Large", "large body")?;

        let metadata = repository.load_document_metadata()?;
        let metadata_node = find_node(&metadata, document.id)?;
        let (content, updated_at) = repository.load_active_node_content(document.id)?;

        assert_eq!(metadata_node.content, "");
        assert_eq!(content, "large body");
        assert_eq!(updated_at, document.updated_at);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn loading_active_node_reads_only_requested_content() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let target =
            repository.create_document_with_content(ROOT_NODE_ID, "Target", "target body")?;
        let other = repository.create_document_with_content(ROOT_NODE_ID, "Other", "other body")?;
        repository.connection.execute(
            "UPDATE nodes SET content = ?1 WHERE id = ?2",
            params!["Bad\0Content", other.id],
        )?;

        let node = repository
            .load_active_node(target.id)?
            .ok_or("target active node was not found")?;

        assert_eq!(node.id, target.id);
        assert_eq!(node.content, "target body");
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn loading_active_node_updated_at_omits_content() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let document =
            repository.create_document_with_content(ROOT_NODE_ID, "Replace Target", "body")?;
        repository.connection.execute(
            "UPDATE nodes SET content = ?1 WHERE id = ?2",
            params!["Bad\0Content", document.id],
        )?;

        let updated_at = repository.load_active_node_updated_at(document.id)?;

        assert_eq!(updated_at, document.updated_at);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn load_document_rejects_stored_embedded_nul_text() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let repository = migrated_repository(&db_path)?;
        repository.connection.execute(
            "UPDATE nodes SET title = ?1 WHERE id = ?2",
            params!["Bad\0Title", DEFAULT_DOCUMENT_ID],
        )?;
        let error = match repository.load_document() {
            Ok(_) => return Err("loading an embedded NUL title succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::EmbeddedNulTitle {
                node_id: DEFAULT_DOCUMENT_ID
            })
        ));

        repository.connection.execute(
            "UPDATE nodes SET title = ?1, content = ?2 WHERE id = ?3",
            params!["Untitled", "Bad\0Content", DEFAULT_DOCUMENT_ID],
        )?;
        let error = match repository.load_document() {
            Ok(_) => return Err("loading embedded NUL content succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::EmbeddedNulContent {
                node_id: DEFAULT_DOCUMENT_ID
            })
        ));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn external_embedded_nul_content_is_not_indexed_for_search() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let document =
            repository.create_document_with_content(ROOT_NODE_ID, "Corruptible", "clean needle")?;
        let indexed_before_corruption: i64 = repository.connection.query_row(
            "SELECT COUNT(*) FROM node_search_fts WHERE rowid = ?1",
            params![document.id],
            |row| row.get(0),
        )?;

        repository.connection.execute(
            "UPDATE nodes SET content = ?1 WHERE id = ?2",
            params!["Bad\0Content", document.id],
        )?;

        let indexed_after_corruption: i64 = repository.connection.query_row(
            "SELECT COUNT(*) FROM node_search_fts WHERE rowid = ?1",
            params![document.id],
            |row| row.get(0),
        )?;

        assert_eq!(indexed_before_corruption, 1);
        assert_eq!(indexed_after_corruption, 0);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn migration_and_initial_content_are_idempotent() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = SqliteDocumentRepository::open(&db_path)?;
        repository.migrate()?;
        let sqlite_schema_version_after_first_migrate: i64 =
            repository
                .connection
                .query_row("PRAGMA schema_version", [], |row| row.get(0))?;
        repository.migrate()?;
        let sqlite_schema_version_after_second_migrate: i64 =
            repository
                .connection
                .query_row("PRAGMA schema_version", [], |row| row.get(0))?;
        repository.ensure_initial_content()?;
        repository.ensure_initial_content()?;

        assert_eq!(
            sqlite_schema_version_after_second_migrate,
            sqlite_schema_version_after_first_migrate
        );

        let migration_count: i64 = repository.connection.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = ?1",
            params![schema::SCHEMA_VERSION],
            |row| row.get(0),
        )?;
        let migration_name: String = repository.connection.query_row(
            "SELECT name FROM schema_migrations WHERE version = ?1",
            params![schema::SCHEMA_VERSION],
            |row| row.get(0),
        )?;
        let settings_table_count: i64 = repository.connection.query_row(
            "
            SELECT COUNT(*)
            FROM sqlite_schema
            WHERE type = 'table' AND name = 'settings'
            ",
            [],
            |row| row.get(0),
        )?;
        let search_table_count: i64 = repository.connection.query_row(
            "
            SELECT COUNT(*)
            FROM sqlite_schema
            WHERE type = 'table' AND name = 'node_search_fts'
            ",
            [],
            |row| row.get(0),
        )?;
        let search_validation_table_count: i64 = repository.connection.query_row(
            "
            SELECT COUNT(*)
            FROM sqlite_schema
            WHERE type = 'table' AND name = 'node_search_validated_content'
            ",
            [],
            |row| row.get(0),
        )?;
        let search_trigger_count: i64 = repository.connection.query_row(
            "
            SELECT COUNT(*)
            FROM sqlite_schema
            WHERE type = 'trigger'
                AND name IN (
                    'node_search_fts_insert',
                    'node_search_fts_update',
                    'node_search_fts_delete'
                )
            ",
            [],
            |row| row.get(0),
        )?;
        let search_index_count: i64 =
            repository
                .connection
                .query_row("SELECT COUNT(*) FROM node_search_fts", [], |row| row.get(0))?;
        let node_count: i64 =
            repository
                .connection
                .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))?;

        assert_eq!(migration_count, 1);
        assert_eq!(migration_name, "node_search_validated_content");
        assert_eq!(settings_table_count, 1);
        assert_eq!(search_table_count, 1);
        assert_eq!(search_validation_table_count, 1);
        assert_eq!(search_trigger_count, 3);
        assert_eq!(search_index_count, 2);
        assert_eq!(node_count, 2);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn concurrent_initial_content_creation_is_idempotent() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = SqliteDocumentRepository::open(&db_path)?;
        repository.migrate()?;
    }

    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let db_path = db_path.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || -> Result<(), String> {
            let mut repository =
                SqliteDocumentRepository::open(&db_path).map_err(|error| error.to_string())?;
            barrier.wait();
            repository
                .ensure_initial_content()
                .map_err(|error| error.to_string())
        }));
    }

    for handle in handles {
        match handle.join() {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => return Err("initial content thread panicked".into()),
        }
    }

    {
        let repository = SqliteDocumentRepository::open(&db_path)?;
        let node_count: i64 =
            repository
                .connection
                .query_row("SELECT COUNT(*) FROM nodes", [], |row| row.get(0))?;

        assert_eq!(node_count, 2);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn migration_rejects_legacy_kind_based_nodes_table() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let connection = rusqlite::Connection::open(&db_path)?;
        connection.execute_batch(
            "
            CREATE TABLE nodes (
                id INTEGER PRIMARY KEY,
                parent_id INTEGER NULL,
                kind TEXT NOT NULL CHECK (kind IN ('folder', 'document')),
                title TEXT NOT NULL CHECK (length(trim(title)) > 0),
                sort_order INTEGER NOT NULL CHECK (sort_order >= 0),
                content TEXT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                CHECK ((kind = 'folder' AND content IS NULL) OR kind = 'document'),
                FOREIGN KEY(parent_id) REFERENCES nodes(id)
            );

            CREATE UNIQUE INDEX idx_nodes_active_sibling_title
                ON nodes(parent_id, title);

            INSERT INTO nodes (
                id, parent_id, kind, title, sort_order, content, created_at, updated_at
            )
            VALUES
                (1, NULL, 'folder', 'Root', 0, NULL, '2026-04-30T00:00:00.000Z', '2026-04-30T00:00:00.000Z'),
                (2, 1, 'document', 'Untitled', 0, '', '2026-04-30T00:00:00.000Z', '2026-04-30T00:00:00.000Z');
            ",
        )?;
    }

    {
        let mut repository = SqliteDocumentRepository::open(&db_path)?;
        let error = match repository.migrate() {
            Ok(()) => return Err("legacy kind based database migration succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(error, AppError::User { .. }));
        assert!(error.user_message().contains("이전 형식"));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn open_database_error_keeps_user_message_on_path_and_permissions() -> Result<(), Box<dyn Error>> {
    let db_path = std::env::temp_dir()
        .join(format!("j3treetext-missing-parent-{}", std::process::id()))
        .join("j3TreeText.db");

    let error = match SqliteDocumentRepository::open(&db_path) {
        Ok(_) => {
            return Err("opening a database under a missing parent unexpectedly worked".into())
        }
        Err(error) => error,
    };

    assert!(matches!(
        &error,
        AppError::DatabaseOpen { path, .. } if path == &db_path
    ));
    let message = error.user_message();
    assert!(message.contains(&db_path.display().to_string()));
    assert!(message.contains("쓰기 권한"));
    Ok(())
}

#[test]
fn create_document_under_document() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let parent = repository.create_child_node(ROOT_NODE_ID, "Projects")?;
        let document = repository.create_child_node(parent.id, "Design Notes")?;
        let model = repository.load_document()?;

        assert!(model.nodes().iter().any(|node| {
            node.id == parent.id
                && node.parent_id == Some(ROOT_NODE_ID)
                && node.title == "Projects"
                && node.content.is_empty()
        }));
        assert!(model.nodes().iter().any(|node| {
            node.id == document.id
                && node.parent_id == Some(parent.id)
                && node.title == "Design Notes"
                && node.content.is_empty()
        }));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn created_children_receive_default_sort_order_and_load_in_display_order(
) -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let gamma = repository.create_child_node(ROOT_NODE_ID, "Gamma")?;
        let alpha = repository.create_child_node(ROOT_NODE_ID, "Alpha")?;

        assert_eq!(gamma.sort_order, 1);
        assert_eq!(alpha.sort_order, 2);

        repository.connection.execute(
            "UPDATE nodes SET sort_order = ?1 WHERE id = ?2",
            params![10, gamma.id],
        )?;
        repository.connection.execute(
            "UPDATE nodes SET sort_order = ?1 WHERE id = ?2",
            params![5, alpha.id],
        )?;

        let model = repository.load_document()?;
        assert_eq!(
            child_ids(&model, Some(ROOT_NODE_ID)),
            vec![DEFAULT_DOCUMENT_ID, alpha.id, gamma.id]
        );
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn create_node_chooses_unique_default_title() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let first = repository.create_child_node(ROOT_NODE_ID, "Draft")?;
        let second = repository.create_child_node(ROOT_NODE_ID, "Draft")?;
        let third = repository.create_child_node(ROOT_NODE_ID, "Draft")?;

        repository.soft_delete_node_cascade(second.id)?;
        let replacement = repository.create_child_node(ROOT_NODE_ID, "Draft")?;

        assert_eq!(first.title, "Draft");
        assert_eq!(second.title, "Draft 2");
        assert_eq!(third.title, "Draft 3");
        assert_eq!(replacement.title, "Draft 2");
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn create_node_chooses_unique_title_with_like_escape_character() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let first = repository.create_child_node(ROOT_NODE_ID, "Use ^ caret")?;
        let second = repository.create_child_node(ROOT_NODE_ID, "Use ^ caret")?;
        let third = repository.create_child_node(ROOT_NODE_ID, "Use ^ caret")?;

        assert_eq!(first.title, "Use ^ caret");
        assert_eq!(second.title, "Use ^ caret 2");
        assert_eq!(third.title, "Use ^ caret 3");
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn create_node_rejects_embedded_nul_title_input() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let error = match repository.create_child_node(ROOT_NODE_ID, "Bad\0Title") {
            Ok(_) => return Err("creating a NUL title node succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::EmbeddedNulTitleInput)
        ));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn concurrent_child_creation_generates_distinct_defaults() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let _repository = migrated_repository(&db_path)?;
    }

    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let db_path = db_path.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || -> Result<Node, String> {
            let mut repository =
                SqliteDocumentRepository::open(&db_path).map_err(|error| error.to_string())?;
            barrier.wait();
            repository
                .create_child_node(ROOT_NODE_ID, "Concurrent")
                .map_err(|error| error.to_string())
        }));
    }

    let mut created = Vec::new();
    for handle in handles {
        match handle.join() {
            Ok(Ok(node)) => created.push(node),
            Ok(Err(error)) => return Err(error.into()),
            Err(_) => return Err("create node thread panicked".into()),
        }
    }

    let mut titles = created
        .iter()
        .map(|node| node.title.as_str())
        .collect::<Vec<_>>();
    titles.sort_unstable();

    let mut sort_orders = created
        .iter()
        .map(|node| node.sort_order)
        .collect::<Vec<_>>();
    sort_orders.sort_unstable();

    assert_ne!(created[0].id, created[1].id);
    assert_eq!(titles, vec!["Concurrent", "Concurrent 2"]);
    assert_eq!(sort_orders, vec![1, 2]);

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn soft_deleted_sibling_title_can_be_reused_and_stays_filtered() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let deleted = repository.create_child_node(ROOT_NODE_ID, "Reusable")?;
        repository.soft_delete_node_cascade(deleted.id)?;

        let replacement = repository.create_child_node(ROOT_NODE_ID, "Reusable")?;
        let active = repository.load_document()?;
        let deleted_nodes = repository.load_deleted_nodes()?;

        assert_eq!(replacement.title, "Reusable");
        assert!(!active.nodes().iter().any(|node| node.id == deleted.id));
        assert!(active.nodes().iter().any(|node| node.id == replacement.id));
        assert!(deleted_nodes.iter().any(|node| node.id == deleted.id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn ui_settings_round_trip_and_invalid_values_fall_back() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let settings = UiSettings {
            window: WindowSettings::new(120, 140, 1000, 700),
            splitter: SplitterSettings::new(360),
            selection: SelectionSettings {
                node_id: Some(DEFAULT_DOCUMENT_ID),
            },
            editor_font: EditorFontSettings::new("Cascadia Mono", 14),
            editor: EditorSettings { word_wrap: false },
            text_encoding: TextEncodingSettings {
                import_encoding: TextEncoding::KoreanEucKr,
                export_encoding: TextEncoding::Utf16BeWithBom,
            },
            appearance: AppearanceSettings {
                theme: AppearanceTheme::Forest,
            },
            language: UiLanguage::English,
            auto_save: AutoSaveSettings::new(true, 300),
        };

        repository.save_ui_settings(&settings)?;
        let loaded = repository.load_ui_settings()?;

        assert_eq!(loaded, settings);

        repository.connection.execute(
            "
            UPDATE settings
            SET value = ?1
            WHERE key IN (
                'window.width',
                'splitter.left_width',
                'selection.node_id',
                'editor.font.size_pt',
                'autosave.enabled',
                'autosave.interval_seconds'
            )
            ",
            params!["not-a-number"],
        )?;
        let loaded = repository.load_ui_settings()?;

        assert_eq!(loaded.window.width, DEFAULT_WINDOW_WIDTH);
        assert_eq!(loaded.window.height, 700);
        assert_eq!(loaded.window.x, Some(120));
        assert_eq!(loaded.window.y, Some(140));
        assert_eq!(loaded.splitter.left_width, DEFAULT_SPLITTER_LEFT_WIDTH);
        assert_eq!(loaded.selection.node_id, None);
        assert_eq!(loaded.editor_font.family, "Cascadia Mono");
        assert_eq!(loaded.editor_font.size_pt, DEFAULT_EDITOR_FONT_SIZE_PT);
        assert_eq!(loaded.auto_save, AutoSaveSettings::default());
        assert_eq!(
            loaded.text_encoding.import_encoding,
            TextEncoding::KoreanEucKr
        );
        assert_eq!(
            loaded.text_encoding.export_encoding,
            TextEncoding::Utf16BeWithBom
        );
        assert_eq!(loaded.language, UiLanguage::English);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn changed_ui_settings_save_writes_only_changed_entries() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let previous = UiSettings {
            window: WindowSettings::new(120, 140, 1000, 700),
            splitter: SplitterSettings::new(360),
            selection: SelectionSettings {
                node_id: Some(DEFAULT_DOCUMENT_ID),
            },
            editor_font: EditorFontSettings::new("Cascadia Mono", 14),
            editor: EditorSettings { word_wrap: false },
            text_encoding: TextEncodingSettings {
                import_encoding: TextEncoding::KoreanEucKr,
                export_encoding: TextEncoding::Utf16BeWithBom,
            },
            appearance: AppearanceSettings {
                theme: AppearanceTheme::Forest,
            },
            language: UiLanguage::English,
            auto_save: AutoSaveSettings::new(true, 300),
        };
        let next = UiSettings {
            splitter: SplitterSettings::new(420),
            selection: SelectionSettings { node_id: None },
            ..previous.clone()
        };
        repository.save_ui_settings(&previous)?;
        repository.connection.execute_batch(
            "
            CREATE TEMP TABLE setting_writes (
                key TEXT NOT NULL
            );
            CREATE TEMP TRIGGER capture_setting_inserts
            AFTER INSERT ON settings
            BEGIN
                INSERT INTO setting_writes (key) VALUES (new.key);
            END;
            CREATE TEMP TRIGGER capture_setting_updates
            AFTER UPDATE ON settings
            BEGIN
                INSERT INTO setting_writes (key) VALUES (new.key);
            END;
            ",
        )?;

        repository.save_changed_ui_settings(&previous, &next)?;

        assert_eq!(
            setting_write_keys(&repository)?,
            vec![
                SETTING_SPLITTER_LEFT_WIDTH.to_owned(),
                SETTING_SELECTION_NODE_ID.to_owned(),
            ]
        );
        let loaded = repository.load_ui_settings()?;
        assert_eq!(loaded.splitter.left_width, 420);
        assert_eq!(loaded.selection.node_id, None);

        repository
            .connection
            .execute("DELETE FROM setting_writes", [])?;
        repository.save_changed_ui_settings(&next, &next)?;

        assert!(setting_write_keys(&repository)?.is_empty());
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn repository_smoke_covers_edit_search_move_trash_restore_and_ui_state(
) -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;
    let selected_node_id;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder_a = repository.create_child_node(ROOT_NODE_ID, "Folder A")?;
        let folder_b = repository.create_child_node(ROOT_NODE_ID, "Folder B")?;
        let document_a = repository.create_child_node(folder_a.id, "Doc A")?;
        let document_b = repository.create_child_node(folder_a.id, "Doc B")?;
        let document_c = repository.create_child_node(folder_b.id, "Doc C")?;
        selected_node_id = document_c.id;

        let loaded = repository.load_document()?;
        let loaded_at = find_node(&loaded, document_a.id)?.updated_at.clone();
        repository.update_document_content(
            document_a.id,
            "integration body\nUTF-8: 한글",
            &loaded_at,
        )?;

        let results = repository.search_documents("integration", 200)?;
        assert!(results.iter().any(|result| result.node.id == document_a.id));

        repository.move_node_to_parent_end(document_b.id, folder_b.id)?;
        let moved = repository.load_document()?;
        assert_eq!(
            find_node(&moved, document_b.id)?.parent_id,
            Some(folder_b.id)
        );

        repository.soft_delete_node_cascade(folder_a.id)?;
        let deleted = repository.load_deleted_nodes()?;
        assert!(deleted.iter().any(|node| node.id == folder_a.id));
        assert!(deleted.iter().any(|node| node.id == document_a.id));

        repository.restore_deleted_node_cascade(folder_a.id)?;
        let restored = repository.load_document()?;
        assert_eq!(
            find_node(&restored, folder_a.id)?.parent_id,
            Some(ROOT_NODE_ID)
        );
        assert_eq!(
            find_node(&restored, document_a.id)?.parent_id,
            Some(folder_a.id)
        );
        assert_eq!(
            find_node(&restored, document_a.id)?.content.as_str(),
            "integration body\nUTF-8: 한글"
        );

        let settings = UiSettings {
            window: WindowSettings::new(200, 220, 1000, 720),
            splitter: SplitterSettings::new(420),
            selection: SelectionSettings {
                node_id: Some(selected_node_id),
            },
            editor_font: EditorFontSettings::new("Consolas", 11),
            editor: EditorSettings { word_wrap: false },
            text_encoding: TextEncodingSettings {
                import_encoding: TextEncoding::Utf8WithBom,
                export_encoding: TextEncoding::Utf16LeWithBom,
            },
            appearance: AppearanceSettings {
                theme: AppearanceTheme::SteelBlue,
            },
            language: UiLanguage::English,
            auto_save: AutoSaveSettings::new(true, 600),
        };
        repository.save_ui_settings(&settings)?;
    }

    {
        let mut reopened = SqliteDocumentRepository::open(&db_path)?;
        reopened.migrate()?;
        reopened.ensure_initial_content()?;

        let loaded = reopened.load_document()?;
        let settings = reopened.load_ui_settings()?;

        assert!(loaded.nodes().iter().any(|node| node.title == "Doc C"));
        assert_eq!(settings.selection.node_id, Some(selected_node_id));
        assert_eq!(settings.splitter.left_width, 420);
        assert_eq!(
            settings.editor_font,
            EditorFontSettings::new("Consolas", 11)
        );
        assert_eq!(
            settings.text_encoding.import_encoding,
            TextEncoding::Utf8WithBom
        );
        assert_eq!(
            settings.text_encoding.export_encoding,
            TextEncoding::Utf16LeWithBom
        );
        assert!(!settings.editor.word_wrap);
        assert_eq!(settings.appearance.theme, AppearanceTheme::SteelBlue);
        assert_eq!(settings.language, UiLanguage::English);
        assert_eq!(settings.auto_save, AutoSaveSettings::new(true, 600));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn rename_rejects_duplicate_sibling_title() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let first = repository.create_child_node(ROOT_NODE_ID, "Alpha")?;
        let second = repository.create_child_node(ROOT_NODE_ID, "Beta")?;
        let error = match repository.rename_node(second.id, "Alpha") {
            Ok(()) => return Err("renaming to a sibling title succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::DuplicateSiblingTitle {
                parent_id: Some(ROOT_NODE_ID),
                title
            }) if title == "Alpha"
        ));

        repository.rename_node(first.id, " Alpha ")?;
        let model = repository.load_document()?;
        assert!(model
            .nodes()
            .iter()
            .any(|node| node.id == first.id && node.title == "Alpha"));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn update_document_content_changes_content_and_updated_at() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        repository.connection.execute(
            "UPDATE nodes SET updated_at = ?1 WHERE id = ?2",
            params!["2000-01-01T00:00:00.000Z", DEFAULT_DOCUMENT_ID],
        )?;

        let saved_at = repository.update_document_content(
            DEFAULT_DOCUMENT_ID,
            "saved body",
            "2000-01-01T00:00:00.000Z",
        )?;
        let model = repository.load_document()?;
        let saved_node = model
            .nodes()
            .iter()
            .find(|node| node.id == DEFAULT_DOCUMENT_ID)
            .ok_or("default document was not found")?;

        assert_eq!(saved_node.content.as_str(), "saved body");
        assert_eq!(saved_node.updated_at, saved_at);
        assert_ne!(saved_node.updated_at, "2000-01-01T00:00:00.000Z");
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn update_document_content_advances_loaded_updated_at_token() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let loaded_updated_at = "2099-01-01T00:00:00.000Z";
        repository.connection.execute(
            "UPDATE nodes SET updated_at = ?1 WHERE id = ?2",
            params![loaded_updated_at, DEFAULT_DOCUMENT_ID],
        )?;

        let saved_at = repository.update_document_content(
            DEFAULT_DOCUMENT_ID,
            "saved after future timestamp",
            loaded_updated_at,
        )?;
        let model = repository.load_document()?;
        let saved_node = find_node(&model, DEFAULT_DOCUMENT_ID)?;

        assert_eq!(saved_at, "2099-01-01T00:00:00.001Z");
        assert_eq!(saved_node.updated_at, saved_at);
        assert_eq!(saved_node.content.as_str(), "saved after future timestamp");
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn document_content_preserves_utf8_text() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let model = repository.load_document()?;
        let mut loaded_updated_at = find_node(&model, DEFAULT_DOCUMENT_ID)?.updated_at.clone();
        let cases = [
            "",
            "ASCII single line",
            "한글 문서",
            "emoji: 🚀",
            "first\r\nsecond\r\n한글 🚀",
            r"{\rtf1\ansi This stays literal plain text}",
        ];

        for body in cases {
            repository.update_document_content(DEFAULT_DOCUMENT_ID, body, &loaded_updated_at)?;
            let (saved, saved_updated_at) =
                repository.load_active_node_content(DEFAULT_DOCUMENT_ID)?;

            assert_eq!(saved.as_str(), body);
            loaded_updated_at = saved_updated_at;
        }
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn update_document_content_rejects_embedded_nul_content() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let model = repository.load_document()?;
        let loaded_updated_at = find_node(&model, DEFAULT_DOCUMENT_ID)?.updated_at.clone();
        let error = match repository.update_document_content(
            DEFAULT_DOCUMENT_ID,
            "Bad\0Content",
            &loaded_updated_at,
        ) {
            Ok(_) => return Err("saving embedded NUL content succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::EmbeddedNulContent {
                node_id: DEFAULT_DOCUMENT_ID
            })
        ));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn update_document_content_detects_stale_updated_at() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let model = repository.load_document()?;
        let loaded_updated_at = model
            .nodes()
            .iter()
            .find(|node| node.id == DEFAULT_DOCUMENT_ID)
            .ok_or("default document was not found")?
            .updated_at
            .clone();

        repository.connection.execute(
            "
            UPDATE nodes
            SET content = ?1, updated_at = ?2
            WHERE id = ?3
            ",
            params![
                "external body",
                "2001-01-01T00:00:00.000Z",
                DEFAULT_DOCUMENT_ID
            ],
        )?;

        let error = match repository.update_document_content(
            DEFAULT_DOCUMENT_ID,
            "local body",
            &loaded_updated_at,
        ) {
            Ok(_) => return Err("stale save unexpectedly succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::DocumentSaveConflict {
                node_id: DEFAULT_DOCUMENT_ID
            })
        ));

        let model = repository.load_document()?;
        let saved_node = model
            .nodes()
            .iter()
            .find(|node| node.id == DEFAULT_DOCUMENT_ID)
            .ok_or("default document was not found")?;

        assert_eq!(saved_node.content.as_str(), "external body");
        assert_eq!(saved_node.updated_at, "2001-01-01T00:00:00.000Z");
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn create_document_with_content_saves_recovered_copy() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let copy =
            repository.create_document_with_content(ROOT_NODE_ID, "Untitled", "local body")?;
        let model = repository.load_document()?;

        assert_ne!(copy.id, DEFAULT_DOCUMENT_ID);
        assert_eq!(copy.title, "Untitled 2");
        assert!(model.nodes().iter().any(|node| {
            node.id == copy.id
                && node.parent_id == Some(ROOT_NODE_ID)
                && node.content == "local body"
        }));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn create_document_with_content_rejects_embedded_nul_content() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let error = match repository.create_document_with_content(
            ROOT_NODE_ID,
            "Recovered",
            "Bad\0Content",
        ) {
            Ok(_) => return Err("creating a recovered copy with embedded NUL succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::EmbeddedNulContent { node_id })
                if node_id > DEFAULT_DOCUMENT_ID
        ));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn search_documents_matches_title_and_content_with_parent_info() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let title_match =
            repository.create_document_with_content(ROOT_NODE_ID, "Release Plan", "body")?;
        let content_match = repository.create_document_with_content(
            ROOT_NODE_ID,
            "Meeting Notes",
            "contains telescope details",
        )?;
        let shared_title_match =
            repository.create_document_with_content(ROOT_NODE_ID, "Shared Needle", "body")?;
        let shared_content_match = repository.create_document_with_content(
            ROOT_NODE_ID,
            "Other Notes",
            "contains shared needle details",
        )?;

        let title_results = repository.search_documents("release", 200)?;
        let content_results = repository.search_documents("telescope", 200)?;
        let shared_results = repository.search_documents("shared needle", 200)?;

        assert!(title_results.iter().any(|result| {
            result.node.id == title_match.id
                && result.node.title == "Release Plan"
                && result.node.content.is_empty()
                && !result.content_matched
                && result.parent_title.as_deref() == Some("Root")
        }));
        assert!(content_results.iter().any(|result| {
            result.node.id == content_match.id
                && result.node.title == "Meeting Notes"
                && result.node.content.is_empty()
                && result.content_matched
                && result.parent_title.as_deref() == Some("Root")
        }));
        assert!(shared_results
            .iter()
            .any(|result| result.node.id == shared_title_match.id));
        assert!(shared_results
            .iter()
            .any(|result| result.node.id == shared_content_match.id));

        repository.update_document_content(
            content_match.id,
            "contains observatory details",
            &content_match.updated_at,
        )?;
        let old_content_results = repository.search_documents("telescope", 200)?;
        let updated_content_results = repository.search_documents("observatory", 200)?;

        assert!(!old_content_results
            .iter()
            .any(|result| result.node.id == content_match.id));
        assert!(updated_content_results
            .iter()
            .any(|result| result.node.id == content_match.id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn search_documents_short_queries_match_titles_then_content_fallback() -> Result<(), Box<dyn Error>>
{
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let first_title = repository.create_document_with_content(ROOT_NODE_ID, "A Alpha", "")?;
        let second_title = repository.create_document_with_content(ROOT_NODE_ID, "A Beta", "")?;
        let title_and_content =
            repository.create_document_with_content(ROOT_NODE_ID, "A Shared", "a")?;
        let content_only = repository.create_document_with_content(
            ROOT_NODE_ID,
            "000 Content",
            "body contains a",
        )?;
        let deleted_content =
            repository.create_document_with_content(ROOT_NODE_ID, "000 Deleted", "a")?;

        repository.soft_delete_node_cascade(deleted_content.id)?;

        let limited_results = repository.search_documents("a", 2)?;
        let all_results = repository.search_documents("a", 200)?;

        assert_eq!(limited_results.len(), 2);
        assert!(limited_results
            .iter()
            .any(|result| result.node.id == first_title.id));
        assert!(limited_results
            .iter()
            .any(|result| result.node.id == second_title.id));
        assert!(all_results
            .iter()
            .any(|result| result.node.id == first_title.id && !result.content_matched));
        assert!(all_results
            .iter()
            .any(|result| result.node.id == title_and_content.id && !result.content_matched));
        assert_eq!(
            all_results
                .iter()
                .filter(|result| result.node.id == title_and_content.id)
                .count(),
            1
        );
        assert!(all_results
            .iter()
            .any(|result| result.node.id == content_only.id && result.content_matched));
        assert!(!all_results
            .iter()
            .any(|result| result.node.id == deleted_content.id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn search_documents_excludes_deleted_documents_and_respects_limit() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let first =
            repository.create_document_with_content(ROOT_NODE_ID, "Common Alpha", "shared")?;
        repository.create_document_with_content(ROOT_NODE_ID, "Common Beta", "shared")?;
        repository.create_document_with_content(ROOT_NODE_ID, "Common Gamma", "shared")?;
        let deleted =
            repository.create_document_with_content(ROOT_NODE_ID, "Common Deleted", "shared")?;

        repository.soft_delete_node_cascade(deleted.id)?;

        let limited_results = repository.search_documents("common", 2)?;
        let deleted_results = repository.search_documents("deleted", 200)?;

        assert_eq!(limited_results.len(), 2);
        assert!(limited_results
            .iter()
            .any(|result| result.node.id == first.id));
        assert!(!deleted_results
            .iter()
            .any(|result| result.node.id == deleted.id));

        repository.restore_deleted_node_cascade(deleted.id)?;
        let restored_results = repository.search_documents("deleted", 200)?;
        assert!(restored_results
            .iter()
            .any(|result| result.node.id == deleted.id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn search_documents_treats_like_wildcards_as_literals() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let percent = repository.create_document_with_content(ROOT_NODE_ID, "Budget 100%", "")?;
        let underscore = repository.create_document_with_content(ROOT_NODE_ID, "A_B Draft", "")?;
        let caret = repository.create_document_with_content(ROOT_NODE_ID, "Use ^ caret", "")?;
        let plain = repository.create_document_with_content(ROOT_NODE_ID, "Budget 1000", "")?;

        let percent_results = repository.search_documents("%", 200)?;
        let underscore_results = repository.search_documents("_", 200)?;
        let caret_results = repository.search_documents("^", 200)?;

        assert!(percent_results
            .iter()
            .any(|result| result.node.id == percent.id));
        assert!(!percent_results
            .iter()
            .any(|result| result.node.id == plain.id));
        assert!(underscore_results
            .iter()
            .any(|result| result.node.id == underscore.id));
        assert!(!underscore_results
            .iter()
            .any(|result| result.node.id == plain.id));
        assert!(caret_results
            .iter()
            .any(|result| result.node.id == caret.id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn soft_delete_root_is_rejected() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let error = match repository.soft_delete_node_cascade(ROOT_NODE_ID) {
            Ok(()) => return Err("soft deleting the root folder succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::CannotDeleteRoot)
        ));
        assert!(repository
            .load_document()?
            .nodes()
            .iter()
            .any(|node| node.id == ROOT_NODE_ID));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn soft_delete_nonstandard_active_root_is_rejected() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    let root_id = ROOT_NODE_ID + 100;
    let child_id = root_id + 1;

    {
        let mut repository = SqliteDocumentRepository::open(&db_path)?;
        repository.migrate()?;

        let transaction = repository.connection.transaction()?;
        let now = current_timestamp(&transaction)?;
        let root = Node {
            id: root_id,
            parent_id: None,
            title: "Imported Root".to_owned(),
            sort_order: 0,
            content: String::new(),
            created_at: now.clone(),
            updated_at: now.clone(),
            deleted_at: None,
        };
        let child = Node {
            id: child_id,
            parent_id: Some(root_id),
            title: "Imported Child".to_owned(),
            sort_order: 0,
            content: String::new(),
            created_at: now.clone(),
            updated_at: now,
            deleted_at: None,
        };
        root.validate()?;
        child.validate()?;
        insert_node(&transaction, &root)?;
        insert_node(&transaction, &child)?;
        transaction.commit()?;

        repository.ensure_initial_content()?;
        let document = repository.load_document()?;
        assert_eq!(find_node(&document, root_id)?.parent_id, None);

        let error = match repository.soft_delete_node_cascade(root_id) {
            Ok(()) => return Err("soft deleting the nonstandard root folder succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::CannotDeleteRoot)
        ));
        assert_eq!(deleted_at_for_node(&repository, root_id)?, None);
        assert!(repository
            .load_document()?
            .nodes()
            .iter()
            .any(|node| node.id == root_id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn soft_delete_folder_cascades_and_load_excludes_deleted() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Archive")?;
        let document = repository.create_child_node(folder.id, "Deleted Note")?;

        repository.soft_delete_node_cascade(folder.id)?;
        let model = repository.load_document()?;

        assert!(!model.nodes().iter().any(|node| node.id == folder.id));
        assert!(!model.nodes().iter().any(|node| node.id == document.id));

        let folder_deleted_at = deleted_at_for_node(&repository, folder.id)?;
        let document_deleted_at = deleted_at_for_node(&repository, document.id)?;
        assert!(folder_deleted_at.is_some());
        assert_eq!(folder_deleted_at, document_deleted_at);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn staged_soft_delete_removes_prepared_active_subtree() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Archive")?;
        let document = repository.create_child_node(folder.id, "Deleted Note")?;

        let staged_node_ids = repository.stage_active_subtree_node_ids_for_delete(folder.id)?;
        assert_eq!(staged_node_ids, vec![folder.id, document.id]);

        let deleted = repository.soft_delete_node_cascade_update_from_staged_active_subtree(
            folder.id,
            &staged_node_ids,
        )?;
        assert_eq!(deleted.removed_node_ids, staged_node_ids);

        let model = repository.load_document()?;
        assert!(!model.nodes().iter().any(|node| node.id == folder.id));
        assert!(!model.nodes().iter().any(|node| node.id == document.id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn staged_soft_delete_falls_back_when_prepared_subtree_is_stale() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Archive")?;
        let document = repository.create_child_node(folder.id, "Deleted Note")?;

        let staged_node_ids = repository.stage_active_subtree_node_ids_for_delete(folder.id)?;
        let late_document = repository.create_child_node(folder.id, "Late Note")?;

        let deleted = repository.soft_delete_node_cascade_update_from_staged_active_subtree(
            folder.id,
            &staged_node_ids,
        )?;
        assert!(deleted.removed_node_ids.contains(&folder.id));
        assert!(deleted.removed_node_ids.contains(&document.id));
        assert!(deleted.removed_node_ids.contains(&late_document.id));

        let model = repository.load_document()?;
        assert!(!model.nodes().iter().any(|node| node.id == folder.id));
        assert!(!model.nodes().iter().any(|node| node.id == document.id));
        assert!(!model.nodes().iter().any(|node| node.id == late_document.id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn soft_delete_recalculates_remaining_sibling_sort_order() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let alpha = repository.create_child_node(ROOT_NODE_ID, "Alpha")?;
        let beta = repository.create_child_node(ROOT_NODE_ID, "Beta")?;
        let gamma = repository.create_child_node(ROOT_NODE_ID, "Gamma")?;

        repository.soft_delete_node_cascade(beta.id)?;
        let model = repository.load_document()?;

        assert_eq!(
            child_sort_orders(&model, Some(ROOT_NODE_ID)),
            vec![(DEFAULT_DOCUMENT_ID, 0), (alpha.id, 1), (gamma.id, 2),]
        );
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn trash_flow_hides_lists_restores_and_permanently_deletes_document() -> Result<(), Box<dyn Error>>
{
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let document =
            repository.create_document_with_content(ROOT_NODE_ID, "Trash Note", "deleted body")?;

        repository.soft_delete_node_cascade(document.id)?;
        let active = repository.load_document()?;
        let trash = repository.load_deleted_nodes()?;

        assert!(!active.nodes().iter().any(|node| node.id == document.id));
        assert!(trash
            .iter()
            .any(|node| node.id == document.id && node.deleted_at.is_some()));
        let deleted_document = repository.load_deleted_node(document.id)?;
        assert_eq!(deleted_document.content, "deleted body");

        repository.restore_deleted_node_cascade(document.id)?;
        let active = repository.load_document()?;
        let trash = repository.load_deleted_nodes()?;

        assert!(active.nodes().iter().any(|node| node.id == document.id));
        assert!(!trash.iter().any(|node| node.id == document.id));

        repository.soft_delete_node_cascade(document.id)?;
        repository.permanently_delete_node_cascade(document.id)?;

        assert_eq!(node_count_in_table(&repository, document.id)?, 0);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn restore_deleted_folder_restores_deleted_descendants() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Archive")?;
        let child = repository.create_child_node(folder.id, "Draft")?;

        repository.soft_delete_node_cascade(folder.id)?;
        repository.restore_deleted_node_cascade(folder.id)?;

        let active = repository.load_document()?;
        let trash = repository.load_deleted_nodes()?;

        assert_eq!(find_node(&active, folder.id)?.parent_id, Some(ROOT_NODE_ID));
        assert_eq!(find_node(&active, child.id)?.parent_id, Some(folder.id));
        assert!(!trash.iter().any(|node| node.id == folder.id));
        assert!(!trash.iter().any(|node| node.id == child.id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn restore_deleted_folder_renames_conflicting_deleted_descendants() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Archive")?;
        let older_child = repository.create_child_node(folder.id, "Draft")?;
        repository.soft_delete_node_cascade(older_child.id)?;
        let current_child = repository.create_child_node(folder.id, "Draft")?;

        repository.soft_delete_node_cascade(folder.id)?;
        repository.restore_deleted_node_cascade(folder.id)?;

        let active = repository.load_document()?;
        let older = find_node(&active, older_child.id)?;
        let current = find_node(&active, current_child.id)?;
        let trash = repository.load_deleted_nodes()?;

        assert_eq!(older.parent_id, Some(folder.id));
        assert_eq!(older.title, "Draft (restored)");
        assert_eq!(current.parent_id, Some(folder.id));
        assert_eq!(current.title, "Draft");
        assert!(!trash.iter().any(|node| node.id == older_child.id));
        assert!(!trash.iter().any(|node| node.id == current_child.id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn restore_child_of_deleted_parent_moves_to_root_and_renames_collision(
) -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Archive")?;
        let child = repository.create_child_node(folder.id, "Draft")?;

        repository.soft_delete_node_cascade(folder.id)?;
        repository.create_child_node(ROOT_NODE_ID, "Draft")?;
        repository.restore_deleted_node_cascade(child.id)?;

        let active = repository.load_document()?;
        let restored = find_node(&active, child.id)?;
        let trash = repository.load_deleted_nodes()?;

        assert_eq!(restored.parent_id, Some(ROOT_NODE_ID));
        assert_eq!(restored.title, "Draft (restored)");
        assert!(!trash.iter().any(|node| node.id == child.id));
        assert!(trash.iter().any(|node| node.id == folder.id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn restore_title_collision_uses_numbered_restored_suffix() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Archive")?;
        let child = repository.create_child_node(folder.id, "Draft")?;

        repository.soft_delete_node_cascade(folder.id)?;
        repository.create_child_node(ROOT_NODE_ID, "Draft")?;
        repository.create_child_node(ROOT_NODE_ID, "Draft (restored)")?;
        repository.restore_deleted_node_cascade(child.id)?;

        let active = repository.load_document()?;
        let restored = find_node(&active, child.id)?;

        assert_eq!(restored.parent_id, Some(ROOT_NODE_ID));
        assert_eq!(restored.title, "Draft (restored) 2");
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn permanently_delete_folder_removes_descendants() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Archive")?;
        let document = repository.create_child_node(folder.id, "Deleted Note")?;

        repository.soft_delete_node_cascade(folder.id)?;
        repository.permanently_delete_node_cascade(folder.id)?;

        assert_eq!(node_count_in_table(&repository, folder.id)?, 0);
        assert_eq!(node_count_in_table(&repository, document.id)?, 0);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn move_document_to_folder_end_returns_changed_sibling_orders() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let document = repository.create_child_node(ROOT_NODE_ID, "Draft")?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Archive")?;

        let updates = repository.move_node_to_parent_end_update(document.id, folder.id)?;
        assert_eq!(
            updates
                .iter()
                .map(|update| (update.node_id, update.parent_id, update.sort_order))
                .collect::<Vec<_>>(),
            vec![
                (folder.id, Some(ROOT_NODE_ID), 1),
                (document.id, Some(folder.id), 0),
            ]
        );

        let model = repository.load_document()?;
        let moved = find_node(&model, document.id)?;

        assert_eq!(moved.parent_id, Some(folder.id));
        assert_eq!(moved.sort_order, 0);
        assert_eq!(
            child_ids(&model, Some(ROOT_NODE_ID)),
            vec![DEFAULT_DOCUMENT_ID, folder.id]
        );
        assert_eq!(child_ids(&model, Some(folder.id)), vec![document.id]);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn move_node_to_same_parent_end_shifts_following_siblings() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let alpha = repository.create_child_node(ROOT_NODE_ID, "Alpha")?;
        let beta = repository.create_child_node(ROOT_NODE_ID, "Beta")?;
        let gamma = repository.create_child_node(ROOT_NODE_ID, "Gamma")?;

        let updates = repository.move_node_to_parent_end_update(alpha.id, ROOT_NODE_ID)?;
        assert_eq!(
            updates
                .iter()
                .map(|update| (update.node_id, update.parent_id, update.sort_order))
                .collect::<Vec<_>>(),
            vec![
                (beta.id, Some(ROOT_NODE_ID), 1),
                (gamma.id, Some(ROOT_NODE_ID), 2),
                (alpha.id, Some(ROOT_NODE_ID), 3),
            ]
        );

        let model = repository.load_document()?;
        assert_eq!(
            child_ids(&model, Some(ROOT_NODE_ID)),
            vec![DEFAULT_DOCUMENT_ID, beta.id, gamma.id, alpha.id]
        );
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn move_folder_to_other_folder_preserves_subtree() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Folder")?;
        let parent = repository.create_child_node(ROOT_NODE_ID, "Parent")?;
        let child = repository.create_child_node(folder.id, "Child")?;

        repository.move_node_to_parent_end(folder.id, parent.id)?;
        let model = repository.load_document()?;

        assert_eq!(find_node(&model, folder.id)?.parent_id, Some(parent.id));
        assert_eq!(find_node(&model, child.id)?.parent_id, Some(folder.id));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn move_folder_rejects_itself_as_parent() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Folder")?;
        let error = match repository.move_node_to_parent_end(folder.id, folder.id) {
            Ok(()) => return Err("moving a folder under itself succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::CannotMoveNodeIntoItself { node_id })
                if node_id == folder.id
        ));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn move_folder_rejects_descendant_parent() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Folder")?;
        let child = repository.create_child_node(folder.id, "Child")?;
        let error = match repository.move_node_to_parent_end(folder.id, child.id) {
            Ok(()) => return Err("moving a folder under its descendant succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::CannotMoveNodeIntoDescendant {
                node_id,
                parent_id
            }) if node_id == folder.id && parent_id == child.id
        ));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn move_node_to_document_end_places_node_under_target() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let alpha = repository.create_child_node(ROOT_NODE_ID, "Alpha")?;
        let beta = repository.create_child_node(ROOT_NODE_ID, "Beta")?;

        repository.move_node_to_parent_end(beta.id, DEFAULT_DOCUMENT_ID)?;
        let model = repository.load_document()?;

        assert_eq!(
            child_ids(&model, Some(ROOT_NODE_ID)),
            vec![DEFAULT_DOCUMENT_ID, alpha.id]
        );
        assert_eq!(child_ids(&model, Some(DEFAULT_DOCUMENT_ID)), vec![beta.id]);
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn move_node_within_parent_swaps_with_neighbor() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let alpha = repository.create_child_node(ROOT_NODE_ID, "Alpha")?;
        let beta = repository.create_child_node(ROOT_NODE_ID, "Beta")?;

        repository.move_node_within_parent(beta.id, SiblingMoveDirection::Up)?;
        let model = repository.load_document()?;

        assert_eq!(
            child_ids(&model, Some(ROOT_NODE_ID)),
            vec![DEFAULT_DOCUMENT_ID, beta.id, alpha.id]
        );
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn mutation_update_methods_return_incremental_changes() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let alpha = repository.create_child_node(ROOT_NODE_ID, "Alpha")?;
        let beta = repository.create_child_node(ROOT_NODE_ID, "Beta")?;

        let renamed = repository.rename_node_update(alpha.id, "Alpha Renamed")?;
        assert_eq!(renamed.title, "Alpha Renamed");
        assert!(!renamed.updated_at.is_empty());

        let moved = repository.move_node_within_parent_update(beta.id, SiblingMoveDirection::Up)?;
        let moved_ids: Vec<i64> = moved.iter().map(|update| update.node_id).collect();
        assert_eq!(moved_ids, vec![beta.id, alpha.id]);
        assert_eq!(
            moved
                .iter()
                .find(|update| update.node_id == beta.id)
                .map(|update| update.sort_order),
            Some(1)
        );

        let deleted = repository.soft_delete_node_cascade_update(beta.id)?;
        assert_eq!(deleted.removed_node_ids, vec![beta.id]);
        let remaining_root_ids: Vec<i64> = deleted
            .sibling_orders
            .iter()
            .map(|update| update.node_id)
            .collect();
        assert_eq!(remaining_root_ids, vec![alpha.id]);
        assert_eq!(
            deleted
                .sibling_orders
                .iter()
                .find(|update| update.node_id == alpha.id)
                .map(|update| update.sort_order),
            Some(1)
        );

        let restored = repository.restore_deleted_node_cascade_update(beta.id)?;
        assert!(restored.iter().any(|node| node.id == beta.id));
        assert!(restored.iter().all(|node| node.deleted_at.is_none()));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn restore_update_returns_metadata_without_loading_content() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let document =
            repository.create_document_with_content(ROOT_NODE_ID, "Large", "large body")?;

        repository.soft_delete_node_cascade(document.id)?;
        let restored = repository.restore_deleted_node_cascade_update(document.id)?;
        let restored_node = restored
            .iter()
            .find(|node| node.id == document.id)
            .ok_or("restored node was not returned")?;
        let (content, _) = repository.load_active_node_content(document.id)?;

        assert_eq!(restored_node.content, "");
        assert_eq!(content, "large body");
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

#[test]
fn move_to_deleted_folder_is_rejected() -> Result<(), Box<dyn Error>> {
    let db_path = unique_test_db_path()?;
    remove_file_if_exists(&db_path)?;

    {
        let mut repository = migrated_repository(&db_path)?;
        let folder = repository.create_child_node(ROOT_NODE_ID, "Trash")?;
        let document = repository.create_child_node(ROOT_NODE_ID, "Movable")?;

        repository.soft_delete_node_cascade(folder.id)?;
        let error = match repository.move_node_to_parent_end(document.id, folder.id) {
            Ok(()) => return Err("moving into a deleted folder succeeded".into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::NodeNotFound { node_id }) if node_id == folder.id
        ));
    }

    remove_file_if_exists(&db_path)?;
    Ok(())
}

fn unique_test_db_path() -> Result<PathBuf, Box<dyn Error>> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    Ok(std::env::temp_dir().join(format!(
        "j3treetext-{}-{nanos}-{counter}.db",
        std::process::id()
    )))
}

fn migrated_repository(db_path: &Path) -> Result<SqliteDocumentRepository, AppError> {
    let mut repository = SqliteDocumentRepository::open(db_path)?;
    repository.migrate()?;
    repository.ensure_initial_content()?;
    Ok(repository)
}

fn deleted_at_for_node(
    repository: &SqliteDocumentRepository,
    node_id: i64,
) -> Result<Option<String>, rusqlite::Error> {
    repository.connection.query_row(
        "SELECT deleted_at FROM nodes WHERE id = ?1",
        params![node_id],
        |row| row.get(0),
    )
}

fn node_count_in_table(
    repository: &SqliteDocumentRepository,
    node_id: i64,
) -> Result<i64, rusqlite::Error> {
    repository.connection.query_row(
        "SELECT COUNT(*) FROM nodes WHERE id = ?1",
        params![node_id],
        |row| row.get(0),
    )
}

fn setting_write_keys(
    repository: &SqliteDocumentRepository,
) -> Result<Vec<String>, rusqlite::Error> {
    let mut statement = repository
        .connection
        .prepare("SELECT key FROM setting_writes ORDER BY rowid")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    let mut keys = Vec::new();
    for row in rows {
        keys.push(row?);
    }
    Ok(keys)
}

fn remove_file_if_exists(path: &Path) -> Result<(), Box<dyn Error>> {
    if path.exists() {
        fs::remove_file(path)?;
    }

    Ok(())
}

fn find_node(document: &Document, node_id: i64) -> Result<&Node, Box<dyn Error>> {
    document
        .nodes()
        .iter()
        .find(|node| node.id == node_id)
        .ok_or_else(|| format!("node {node_id} was not found").into())
}

fn child_ids(document: &Document, parent_id: Option<i64>) -> Vec<i64> {
    document
        .nodes()
        .iter()
        .filter(|node| node.parent_id == parent_id)
        .map(|node| node.id)
        .collect()
}

fn child_sort_orders(document: &Document, parent_id: Option<i64>) -> Vec<(i64, i64)> {
    document
        .nodes()
        .iter()
        .filter(|node| node.parent_id == parent_id)
        .map(|node| (node.id, node.sort_order))
        .collect()
}
