use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::Document;
use crate::domain::DocumentSearchResult;
use crate::domain::DomainError;
use crate::domain::Node;
use crate::domain::SiblingMoveDirection;
use crate::domain::TextEncoding;
use crate::domain::UiSettings;
use crate::domain::APP_DISPLAY_NAME;
use crate::domain::SEARCH_RESULT_LIMIT;
use crate::error::AppError;
use crate::infra::sqlite::{
    database_path_for_current_exe, DeleteNodeUpdate, SqliteDocumentRepository,
};
#[cfg(test)]
use crate::infra::text_file::encode_text_file_for_export;
use crate::infra::text_file::{read_text_file, write_text_file, DecodedText};
#[cfg(windows)]
use crate::infra::text_file::{write_encoded_text_file, EncodedTextExport};
use crate::infra::text_tree_export::write_document_tree_text_files_with_content_loader;

const DEFAULT_NEW_DOCUMENT_TITLE: &str = "Untitled";

pub struct App {
    db_path: PathBuf,
    repository: SqliteDocumentRepository,
    document: Document,
    ui_settings: UiSettings,
}

pub(crate) struct BootstrappedDocumentRepository {
    db_path: PathBuf,
    repository: SqliteDocumentRepository,
}

impl BootstrappedDocumentRepository {
    pub(crate) fn open(db_path: Option<&Path>) -> Result<Self, AppError> {
        let db_path = match db_path {
            Some(path) => path.to_path_buf(),
            None => database_path_for_current_exe()?,
        };
        let mut repository = SqliteDocumentRepository::open(&db_path)?;
        repository.migrate()?;
        repository.ensure_initial_content()?;

        Ok(Self {
            db_path,
            repository,
        })
    }

    pub(crate) fn into_repository(self) -> SqliteDocumentRepository {
        self.repository
    }
}

impl App {
    pub fn start() -> Result<Self, AppError> {
        Self::start_with_database_path(None)
    }

    pub fn start_with_database_path(db_path: Option<&Path>) -> Result<Self, AppError> {
        let bootstrapped = BootstrappedDocumentRepository::open(db_path)?;
        let BootstrappedDocumentRepository {
            db_path,
            repository,
        } = bootstrapped;
        let document = repository.load_document_metadata()?;
        let ui_settings = repository.load_ui_settings()?;

        Ok(Self {
            db_path,
            repository,
            document,
            ui_settings,
        })
    }

    pub fn window_title(&self) -> &'static str {
        APP_DISPLAY_NAME
    }

    pub fn database_path(&self) -> &Path {
        &self.db_path
    }

    pub fn document(&self) -> &Document {
        &self.document
    }

    pub fn ui_settings(&self) -> UiSettings {
        self.ui_settings.clone()
    }

    pub fn ui_settings_ref(&self) -> &UiSettings {
        &self.ui_settings
    }

    pub fn node_count(&self) -> usize {
        self.document.node_count()
    }

    pub fn load_active_node_content(&self, node_id: i64) -> Result<(String, String), AppError> {
        self.repository.load_active_node_content(node_id)
    }

    pub fn load_active_node_content_if_present(
        &self,
        node_id: i64,
    ) -> Result<Option<(String, String)>, AppError> {
        self.repository.load_active_node_content_if_present(node_id)
    }

    pub fn load_active_node_contents_if_present(
        &self,
        node_ids: &[i64],
    ) -> Result<HashMap<i64, (String, String)>, AppError> {
        self.repository
            .load_active_node_contents_if_present(node_ids)
    }

    pub fn load_deleted_node_content(&self, node_id: i64) -> Result<(String, String), AppError> {
        self.repository.load_deleted_node_content(node_id)
    }

    pub fn load_deleted_node_contents(
        &self,
        node_ids: &[i64],
    ) -> Result<HashMap<i64, (String, String)>, AppError> {
        self.repository.load_deleted_node_contents(node_ids)
    }

    pub fn create_document(&mut self, parent_id: i64) -> Result<i64, AppError> {
        self.create_child_node(parent_id, DEFAULT_NEW_DOCUMENT_TITLE)
    }

    pub fn rename_node(&mut self, node_id: i64, title: &str) -> Result<(), AppError> {
        let update = self.repository.rename_node_update(node_id, title)?;
        self.apply_committed_document_update(|document| {
            document.rename_node(node_id, update.title, update.updated_at)
        })
    }

    pub fn save_document_content(
        &mut self,
        node_id: i64,
        content: &str,
        expected_updated_at: &str,
    ) -> Result<String, AppError> {
        let updated_at =
            self.repository
                .update_document_content(node_id, content, expected_updated_at)?;
        self.apply_committed_document_update(|document| {
            document.replace_node_content(node_id, String::new(), updated_at.clone())
        })?;
        Ok(updated_at)
    }

    pub fn save_document_content_as_new_document(
        &mut self,
        parent_id: i64,
        base_title: &str,
        content: &str,
    ) -> Result<i64, AppError> {
        let node = self
            .repository
            .create_document_with_content(parent_id, base_title, content)?;
        let node_id = node.id;
        self.apply_committed_document_update(|document| document.insert_node(node))?;
        Ok(node_id)
    }

    pub fn reload_document(&mut self) -> Result<(), AppError> {
        self.document = self.repository.load_document_metadata()?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn from_repository_for_test(
        db_path: PathBuf,
        repository: SqliteDocumentRepository,
    ) -> Result<Self, AppError> {
        let document = repository.load_document_metadata()?;
        let ui_settings = repository.load_ui_settings()?;

        Ok(Self {
            db_path,
            repository,
            document,
            ui_settings,
        })
    }

    pub fn active_subtree_node_ids(&self, node_id: i64) -> Result<Vec<i64>, AppError> {
        self.repository.load_active_subtree_node_ids(node_id)
    }

    pub fn stage_active_subtree_node_ids_for_delete(
        &mut self,
        node_id: i64,
    ) -> Result<Vec<i64>, AppError> {
        self.repository
            .stage_active_subtree_node_ids_for_delete(node_id)
    }

    pub fn delete_node(&mut self, node_id: i64) -> Result<Vec<i64>, AppError> {
        let update = self.repository.soft_delete_node_cascade_update(node_id)?;
        self.apply_delete_node_update(update)
    }

    pub fn delete_node_from_staged_active_subtree(
        &mut self,
        node_id: i64,
        staged_node_ids: &[i64],
    ) -> Result<Vec<i64>, AppError> {
        let update = self
            .repository
            .soft_delete_node_cascade_update_from_staged_active_subtree(node_id, staged_node_ids)?;
        self.apply_delete_node_update(update)
    }

    fn apply_delete_node_update(&mut self, update: DeleteNodeUpdate) -> Result<Vec<i64>, AppError> {
        let removed_node_ids = update.removed_node_ids;
        let sibling_orders = update.sibling_orders;
        self.apply_committed_document_update(|document| {
            document
                .remove_nodes_and_apply_sibling_order_updates(&removed_node_ids, &sibling_orders)
        })?;
        Ok(removed_node_ids)
    }

    pub fn deleted_nodes(&self) -> Result<Vec<Node>, AppError> {
        self.repository.load_deleted_nodes()
    }

    pub fn search_documents(&self, query: &str) -> Result<Vec<DocumentSearchResult>, AppError> {
        self.repository.search_documents(query, SEARCH_RESULT_LIMIT)
    }

    pub fn restore_node(&mut self, node_id: i64) -> Result<(), AppError> {
        let nodes = self
            .repository
            .restore_deleted_node_cascade_update(node_id)?;
        self.apply_committed_document_update(|document| document.insert_nodes(nodes))
    }

    pub fn permanently_delete_node(&mut self, node_id: i64) -> Result<(), AppError> {
        self.repository.permanently_delete_node_cascade(node_id)?;
        Ok(())
    }

    pub fn move_node_to_parent_end(
        &mut self,
        node_id: i64,
        parent_id: i64,
    ) -> Result<(), AppError> {
        let updates = self
            .repository
            .move_node_to_parent_end_update(node_id, parent_id)?;
        self.apply_committed_document_update(|document| {
            document.apply_sibling_order_updates(&updates)
        })
    }

    pub fn move_node_within_parent(
        &mut self,
        node_id: i64,
        direction: SiblingMoveDirection,
    ) -> Result<(), AppError> {
        let updates = self
            .repository
            .move_node_within_parent_update(node_id, direction)?;
        self.apply_committed_document_update(|document| {
            document.apply_sibling_order_updates(&updates)
        })
    }

    pub fn save_ui_settings(&mut self, ui_settings: UiSettings) -> Result<(), AppError> {
        if ui_settings == self.ui_settings {
            return Ok(());
        }

        self.repository
            .save_changed_ui_settings(&self.ui_settings, &ui_settings)?;
        self.ui_settings = ui_settings;
        Ok(())
    }

    pub fn import_text_file(
        &self,
        path: &Path,
        encoding: TextEncoding,
    ) -> Result<DecodedText, AppError> {
        read_text_file(path, encoding)
    }

    pub fn export_text_file(
        &self,
        path: &Path,
        encoding: TextEncoding,
        content: &str,
    ) -> Result<(), AppError> {
        write_text_file(path, encoding, content)
    }

    pub fn export_all_text_files(
        &self,
        directory: &Path,
        encoding: TextEncoding,
        content_overrides: &HashMap<i64, &str>,
    ) -> Result<usize, AppError> {
        let node_ids = self
            .document
            .nodes()
            .iter()
            .filter(|node| !content_overrides.contains_key(&node.id))
            .map(|node| node.id)
            .collect::<Vec<_>>();
        let contents = self
            .repository
            .load_active_node_contents_if_present(&node_ids)?;

        write_document_tree_text_files_with_content_loader(
            directory,
            &self.document,
            encoding,
            content_overrides,
            |node| {
                let (content, _) = contents
                    .get(&node.id)
                    .ok_or(DomainError::NodeNotFound { node_id: node.id })?;
                Ok(Cow::Borrowed(content.as_str()))
            },
        )
    }

    #[cfg(windows)]
    pub(crate) fn export_encoded_text_file(
        &self,
        path: &Path,
        export: &EncodedTextExport,
    ) -> Result<(), AppError> {
        write_encoded_text_file(path, export)
    }

    fn create_child_node(&mut self, parent_id: i64, default_title: &str) -> Result<i64, AppError> {
        let node = self
            .repository
            .create_child_node(parent_id, default_title)?;
        let node_id = node.id;
        self.apply_committed_document_update(|document| document.insert_node(node))?;
        Ok(node_id)
    }

    fn apply_committed_document_update(
        &mut self,
        apply: impl FnOnce(&mut Document) -> Result<(), DomainError>,
    ) -> Result<(), AppError> {
        match apply(&mut self.document) {
            Ok(()) => Ok(()),
            Err(_) => self.reload_document(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::domain::{DEFAULT_DOCUMENT_ID, ROOT_NODE_ID};

    #[test]
    fn reload_document_omits_content_until_content_reload_requested() -> Result<(), Box<dyn Error>>
    {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        {
            let mut repository = SqliteDocumentRepository::open(&db_path)?;
            repository.migrate()?;
            repository.ensure_initial_content()?;
            let node =
                repository.create_document_with_content(ROOT_NODE_ID, "Large", "large body")?;
            let document = repository.load_document()?;
            let ui_settings = repository.load_ui_settings()?;
            let mut app = App {
                db_path: db_path.clone(),
                repository,
                document,
                ui_settings,
            };

            assert_eq!(node_content(app.document(), node.id)?, "large body");

            app.reload_document()?;
            assert_eq!(node_content(app.document(), node.id)?, "");

            let (content, updated_at) = app.load_active_node_content(node.id)?;
            assert_eq!(content, "large body");
            assert_eq!(updated_at, node.updated_at);
        }

        remove_file_if_exists(&db_path)?;
        Ok(())
    }

    #[test]
    fn content_loaders_keep_active_and_deleted_rows_separate() -> Result<(), Box<dyn Error>> {
        with_test_app(|app, db_path| {
            let mut external = SqliteDocumentRepository::open(db_path)?;
            let node =
                external.create_document_with_content(ROOT_NODE_ID, "Deleted Content", "body")?;
            app.reload_document()?;
            external.soft_delete_node_cascade(node.id)?;

            assert!(app.load_active_node_content_if_present(node.id)?.is_none());
            assert!(!app
                .load_active_node_contents_if_present(&[node.id])?
                .contains_key(&node.id));
            assert!(app.load_active_node_content(node.id).is_err());

            let (content, _) = app.load_deleted_node_content(node.id)?;
            assert_eq!(content, "body");
            let deleted_contents = app.load_deleted_node_contents(&[node.id])?;
            assert_eq!(
                deleted_contents
                    .get(&node.id)
                    .map(|content| content.0.as_str()),
                Some("body")
            );
            Ok(())
        })
    }

    #[test]
    fn rename_node_resyncs_after_committed_update_misses_stale_document(
    ) -> Result<(), Box<dyn Error>> {
        with_test_app(|app, db_path| {
            let mut external = SqliteDocumentRepository::open(db_path)?;
            let external_node =
                external.create_document_with_content(ROOT_NODE_ID, "External", "")?;

            app.rename_node(external_node.id, "Renamed externally visible")?;

            assert_eq!(
                node_title(app.document(), external_node.id)?,
                "Renamed externally visible"
            );
            Ok(())
        })
    }

    #[test]
    fn save_document_content_resyncs_after_committed_update_misses_stale_document(
    ) -> Result<(), Box<dyn Error>> {
        with_test_app(|app, db_path| {
            let mut external = SqliteDocumentRepository::open(db_path)?;
            let external_node =
                external.create_document_with_content(ROOT_NODE_ID, "External Save", "old")?;

            let updated_at =
                app.save_document_content(external_node.id, "new", &external_node.updated_at)?;

            let (content, loaded_updated_at) = app.load_active_node_content(external_node.id)?;
            assert_eq!(content, "new");
            assert_eq!(loaded_updated_at, updated_at);
            assert_eq!(node_content(app.document(), external_node.id)?, "");
            Ok(())
        })
    }

    #[test]
    fn save_document_content_keeps_document_cache_metadata_only() -> Result<(), Box<dyn Error>> {
        with_test_app(|app, _| {
            let (_, loaded_updated_at) = app.load_active_node_content(DEFAULT_DOCUMENT_ID)?;
            let large_body = "large body\n".repeat(8192);

            let saved_updated_at =
                app.save_document_content(DEFAULT_DOCUMENT_ID, &large_body, &loaded_updated_at)?;

            let cached_node = node_by_id(app.document(), DEFAULT_DOCUMENT_ID)?;
            assert_eq!(cached_node.content.as_str(), "");
            assert_eq!(cached_node.updated_at.as_str(), saved_updated_at.as_str());

            let (content, reloaded_updated_at) =
                app.load_active_node_content(DEFAULT_DOCUMENT_ID)?;
            assert_eq!(content, large_body);
            assert_eq!(reloaded_updated_at, saved_updated_at);
            Ok(())
        })
    }

    #[test]
    fn save_document_content_reopens_hangul_emoji_and_crlf_plain_text() -> Result<(), Box<dyn Error>>
    {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;
        let saved_body = "첫줄 한글\r\nemoji 🚀\r\n끝";

        {
            let mut app = create_test_app(&db_path)?;
            let (_, loaded_updated_at) = app.load_active_node_content(DEFAULT_DOCUMENT_ID)?;

            let saved_updated_at =
                app.save_document_content(DEFAULT_DOCUMENT_ID, saved_body, &loaded_updated_at)?;
            let (content, reopened_updated_at) =
                app.load_active_node_content(DEFAULT_DOCUMENT_ID)?;

            assert_eq!(content, saved_body);
            assert_eq!(reopened_updated_at, saved_updated_at);
        }

        {
            let reopened = create_test_app(&db_path)?;
            let (content, _) = reopened.load_active_node_content(DEFAULT_DOCUMENT_ID)?;

            assert_eq!(content, saved_body);
        }

        remove_file_if_exists(&db_path)?;
        Ok(())
    }

    #[test]
    fn export_all_text_files_writes_active_tree_with_overrides() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        let export_dir = unique_test_dir("export-all")?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let mut app = create_test_app(&db_path)?;
            let exported_id =
                app.save_document_content_as_new_document(ROOT_NODE_ID, "Exported", "stored")?;
            let mut overrides = std::collections::HashMap::new();
            overrides.insert(exported_id, "dirty export");

            let count = app.export_all_text_files(&export_dir, TextEncoding::Utf8, &overrides)?;

            assert_eq!(count, app.document().node_count());
            assert_eq!(
                fs::read_to_string(
                    export_dir
                        .join("1 - Root")
                        .join(format!("{exported_id} - Exported.txt"))
                )?,
                "dirty export"
            );
            Ok(())
        })();
        let file_cleanup = remove_file_if_exists(&db_path);
        let dir_cleanup = remove_dir_if_exists(&export_dir);

        result?;
        file_cleanup?;
        dir_cleanup?;
        Ok(())
    }

    #[test]
    fn export_text_file_writes_prepared_export_bytes() -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        let export_dir = unique_test_dir("export-text")?;
        remove_file_if_exists(&db_path)?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let app = create_test_app(&db_path)?;
            fs::create_dir(&export_dir)?;
            let cases = [
                (TextEncoding::Utf8, "utf8.txt", "plain 한글"),
                (TextEncoding::Utf8WithBom, "utf8-bom.txt", "bom 한글"),
                (TextEncoding::KoreanEucKr, "euc-kr.txt", "한글"),
            ];

            for (encoding, file_name, content) in cases {
                let path = export_dir.join(file_name);

                app.export_text_file(&path, encoding, content)?;

                let expected = encode_text_file_for_export(content, encoding)?;
                assert_eq!(fs::read(&path)?, expected.as_bytes());
            }
            Ok(())
        })();
        let file_cleanup = remove_file_if_exists(&db_path);
        let dir_cleanup = remove_dir_if_exists(&export_dir);

        result?;
        file_cleanup?;
        dir_cleanup?;
        Ok(())
    }

    #[test]
    fn delete_node_resyncs_after_committed_update_misses_stale_document(
    ) -> Result<(), Box<dyn Error>> {
        with_test_app(|app, db_path| {
            let mut external = SqliteDocumentRepository::open(db_path)?;
            let external_node =
                external.create_document_with_content(ROOT_NODE_ID, "External Delete", "")?;

            app.delete_node(external_node.id)?;

            assert!(app.document().node_by_id(external_node.id).is_none());
            assert!(app
                .deleted_nodes()?
                .iter()
                .any(|node| node.id == external_node.id));
            Ok(())
        })
    }

    #[test]
    fn delete_node_returns_actual_removed_subtree_from_repository() -> Result<(), Box<dyn Error>> {
        with_test_app(|app, db_path| {
            let parent_id =
                app.save_document_content_as_new_document(ROOT_NODE_ID, "Stale Parent", "")?;
            let mut external = SqliteDocumentRepository::open(db_path)?;
            let external_child =
                external.create_document_with_content(parent_id, "External Child", "")?;

            let active_subtree_node_ids = app.active_subtree_node_ids(parent_id)?;
            assert!(active_subtree_node_ids.contains(&parent_id));
            assert!(active_subtree_node_ids.contains(&external_child.id));

            let removed_node_ids = app.delete_node(parent_id)?;

            assert!(removed_node_ids.contains(&parent_id));
            assert!(removed_node_ids.contains(&external_child.id));
            assert!(app.document().node_by_id(parent_id).is_none());
            assert!(app.document().node_by_id(external_child.id).is_none());
            Ok(())
        })
    }

    #[test]
    fn restore_node_resyncs_after_committed_update_duplicates_stale_document(
    ) -> Result<(), Box<dyn Error>> {
        with_test_app(|app, db_path| {
            let existing_node = {
                let mut external = SqliteDocumentRepository::open(db_path)?;
                let node =
                    external.create_document_with_content(ROOT_NODE_ID, "External Restore", "")?;
                app.reload_document()?;
                external.soft_delete_node_cascade(node.id)?;
                node
            };

            app.restore_node(existing_node.id)?;

            assert_eq!(
                node_title(app.document(), existing_node.id)?,
                "External Restore"
            );
            Ok(())
        })
    }

    #[test]
    fn move_node_within_parent_resyncs_after_committed_update_mentions_missing_stale_sibling(
    ) -> Result<(), Box<dyn Error>> {
        with_test_app(|app, db_path| {
            let mut external = SqliteDocumentRepository::open(db_path)?;
            let external_node =
                external.create_document_with_content(ROOT_NODE_ID, "External Move", "")?;

            app.move_node_within_parent(DEFAULT_DOCUMENT_ID, SiblingMoveDirection::Down)?;

            assert!(
                node_sort_order(app.document(), external_node.id)?
                    < node_sort_order(app.document(), DEFAULT_DOCUMENT_ID)?
            );
            Ok(())
        })
    }

    #[test]
    fn move_node_to_parent_end_resyncs_after_committed_update_would_cycle_stale_document(
    ) -> Result<(), Box<dyn Error>> {
        with_test_app(|app, db_path| {
            let parent_id =
                app.save_document_content_as_new_document(ROOT_NODE_ID, "Stale Parent", "")?;
            let child_id =
                app.save_document_content_as_new_document(parent_id, "Stale Child", "")?;
            let mut external = SqliteDocumentRepository::open(db_path)?;
            external.move_node_to_parent_end(child_id, ROOT_NODE_ID)?;
            external.move_node_to_parent_end(parent_id, ROOT_NODE_ID)?;

            app.move_node_to_parent_end(parent_id, child_id)?;

            assert_eq!(node_parent_id(app.document(), parent_id)?, Some(child_id));
            assert_eq!(
                node_parent_id(app.document(), child_id)?,
                Some(ROOT_NODE_ID)
            );
            Ok(())
        })
    }

    #[test]
    fn create_document_resyncs_after_committed_update_duplicates_stale_document(
    ) -> Result<(), Box<dyn Error>> {
        with_test_app(|app, db_path| {
            let mut external = SqliteDocumentRepository::open(db_path)?;
            external.soft_delete_node_cascade(DEFAULT_DOCUMENT_ID)?;
            external.permanently_delete_node_cascade(DEFAULT_DOCUMENT_ID)?;

            let created_id = app.create_document(ROOT_NODE_ID)?;

            assert_eq!(created_id, DEFAULT_DOCUMENT_ID);
            assert_eq!(
                node_title(app.document(), created_id)?,
                DEFAULT_NEW_DOCUMENT_TITLE
            );
            Ok(())
        })
    }

    fn unique_test_db_path() -> Result<PathBuf, Box<dyn Error>> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        Ok(std::env::temp_dir().join(format!(
            "j3treetext-app-{}-{nanos}-{counter}.db",
            std::process::id()
        )))
    }

    fn unique_test_dir(name: &str) -> Result<PathBuf, Box<dyn Error>> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        Ok(std::env::temp_dir().join(format!(
            "j3treetext-app-{name}-{}-{nanos}-{counter}",
            std::process::id()
        )))
    }

    fn remove_file_if_exists(path: &Path) -> Result<(), Box<dyn Error>> {
        if path.exists() {
            fs::remove_file(path)?;
        }

        Ok(())
    }

    fn remove_dir_if_exists(path: &Path) -> Result<(), Box<dyn Error>> {
        if path.exists() {
            fs::remove_dir_all(path)?;
        }

        Ok(())
    }

    fn with_test_app(
        test: impl FnOnce(&mut App, &Path) -> Result<(), Box<dyn Error>>,
    ) -> Result<(), Box<dyn Error>> {
        let db_path = unique_test_db_path()?;
        remove_file_if_exists(&db_path)?;

        let mut app = create_test_app(&db_path)?;
        let result = test(&mut app, &db_path);
        drop(app);
        let cleanup = remove_file_if_exists(&db_path);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    fn create_test_app(db_path: &Path) -> Result<App, Box<dyn Error>> {
        let mut repository = SqliteDocumentRepository::open(db_path)?;
        repository.migrate()?;
        repository.ensure_initial_content()?;
        let document = repository.load_document_metadata()?;
        let ui_settings = repository.load_ui_settings()?;

        Ok(App {
            db_path: db_path.to_path_buf(),
            repository,
            document,
            ui_settings,
        })
    }

    fn node_title(document: &Document, node_id: i64) -> Result<&str, Box<dyn Error>> {
        node_by_id(document, node_id).map(|node| node.title.as_str())
    }

    fn node_content(document: &Document, node_id: i64) -> Result<&str, Box<dyn Error>> {
        node_by_id(document, node_id).map(|node| node.content.as_str())
    }

    fn node_sort_order(document: &Document, node_id: i64) -> Result<i64, Box<dyn Error>> {
        node_by_id(document, node_id).map(|node| node.sort_order)
    }

    fn node_parent_id(document: &Document, node_id: i64) -> Result<Option<i64>, Box<dyn Error>> {
        node_by_id(document, node_id).map(|node| node.parent_id)
    }

    fn node_by_id(document: &Document, node_id: i64) -> Result<&Node, Box<dyn Error>> {
        document
            .nodes()
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| format!("node {node_id} was not found").into())
    }
}
