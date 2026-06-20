use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::{Document, Node, TextEncoding};
use crate::error::{AppError, IoUserMessage};
use crate::platform::file_system;

use super::text_file::{
    encode_text_file_for_export, validate_text_file_export_encoding,
    write_encoded_text_export_to_file, EncodedTextExport,
};

const DEFAULT_EXPORT_TITLE: &str = "Untitled";
const MAX_EXPORT_TITLE_CHARS: usize = 96;
const REPLACEMENT_FILE_ATTEMPTS: u32 = 100;

struct PlannedTextExport<'a> {
    node: &'a Node,
    directory: PathBuf,
    file_path: PathBuf,
}

struct PreparedTextExport<'a> {
    file_path: PathBuf,
    content: Cow<'a, str>,
}

#[cfg(test)]
fn write_document_tree_text_files(
    destination: &Path,
    document: &Document,
    encoding: TextEncoding,
    content_overrides: &HashMap<i64, &str>,
) -> Result<usize, AppError> {
    write_document_tree_text_files_with_content_loader(
        destination,
        document,
        encoding,
        content_overrides,
        |node| Ok(Cow::Borrowed(node.content.as_str())),
    )
}

pub(crate) fn write_document_tree_text_files_with_content_loader<'a>(
    destination: &Path,
    document: &'a Document,
    encoding: TextEncoding,
    content_overrides: &'a HashMap<i64, &'a str>,
    mut load_content: impl FnMut(&'a Node) -> Result<Cow<'a, str>, AppError>,
) -> Result<usize, AppError> {
    let exports = plan_document_tree_text_exports(destination, document.nodes());
    if exports.is_empty() {
        return Ok(0);
    }

    let directories = export_directories(&exports);
    if text_encoding_needs_prevalidation(encoding) {
        let prepared_exports = prepare_document_tree_text_exports(
            &exports,
            encoding,
            content_overrides,
            &mut load_content,
        )?;
        create_document_tree_export_directories(&directories)?;
        let export_count = prepared_exports.len();
        let mut write_session = TextTreeExportWriteSession::new();
        let result = write_prepared_text_exports(prepared_exports, encoding, &mut write_session);
        finish_text_tree_export_write(result, &mut write_session)?;
        return Ok(export_count);
    }

    create_document_tree_export_directories(&directories)?;

    let export_count = exports.len();
    let mut write_session = TextTreeExportWriteSession::new();
    let result = write_document_tree_text_exports_inner(
        exports,
        encoding,
        content_overrides,
        &mut load_content,
        &mut write_session,
    );
    finish_text_tree_export_write(result, &mut write_session)?;

    Ok(export_count)
}

fn prepare_document_tree_text_exports<'a>(
    exports: &[PlannedTextExport<'a>],
    encoding: TextEncoding,
    content_overrides: &'a HashMap<i64, &'a str>,
    load_content: &mut impl FnMut(&'a Node) -> Result<Cow<'a, str>, AppError>,
) -> Result<Vec<PreparedTextExport<'a>>, AppError> {
    let mut prepared_exports = Vec::with_capacity(exports.len());
    for export in exports {
        let content = export_content(export.node, content_overrides, load_content)?;
        validate_text_file_export_encoding(&content, encoding)?;
        prepared_exports.push(PreparedTextExport {
            file_path: export.file_path.clone(),
            content,
        });
    }
    Ok(prepared_exports)
}

fn create_document_tree_export_directories(
    directories: &BTreeSet<PathBuf>,
) -> Result<(), AppError> {
    for directory in directories {
        fs::create_dir_all(directory).map_err(|source| {
            AppError::io_with_user_message(
                "create document tree export directory",
                IoUserMessage::WriteTextFile,
                source,
            )
        })?;
    }
    Ok(())
}

fn write_document_tree_text_exports_inner<'a>(
    exports: Vec<PlannedTextExport<'a>>,
    encoding: TextEncoding,
    content_overrides: &'a HashMap<i64, &'a str>,
    load_content: &mut impl FnMut(&'a Node) -> Result<Cow<'a, str>, AppError>,
    write_session: &mut TextTreeExportWriteSession,
) -> Result<(), AppError> {
    for export in exports {
        let content = export_content(export.node, content_overrides, load_content)?;
        let encoded = encode_text_file_for_export(&content, encoding)?;
        write_session.replace_with_encoded_file_atomically(&export.file_path, &encoded)?;
    }
    Ok(())
}

fn write_prepared_text_exports(
    exports: Vec<PreparedTextExport<'_>>,
    encoding: TextEncoding,
    write_session: &mut TextTreeExportWriteSession,
) -> Result<(), AppError> {
    for export in exports {
        let encoded = encode_text_file_for_export(&export.content, encoding)?;
        write_session.replace_with_encoded_file_atomically(&export.file_path, &encoded)?;
    }
    Ok(())
}

fn export_directories(exports: &[PlannedTextExport<'_>]) -> BTreeSet<PathBuf> {
    exports
        .iter()
        .map(|export| export.directory.clone())
        .collect()
}

fn text_encoding_needs_prevalidation(encoding: TextEncoding) -> bool {
    !matches!(encoding, TextEncoding::Utf8)
}

struct TextTreeExportWriteSession {
    #[cfg(unix)]
    dirty_parent_directories: BTreeSet<PathBuf>,
}

impl TextTreeExportWriteSession {
    fn new() -> Self {
        Self {
            #[cfg(unix)]
            dirty_parent_directories: BTreeSet::new(),
        }
    }

    fn replace_with_encoded_file_atomically(
        &mut self,
        file_path: &Path,
        export: &EncodedTextExport,
    ) -> Result<(), AppError> {
        let (temp_path, file) = create_temp_replacement_file(file_path)?;
        let mut file = match file_system::prepare_replacement_file(&temp_path, file_path, file) {
            Ok(file) => file,
            Err(source) => {
                let _ = fs::remove_file(&temp_path);
                return Err(AppError::io(
                    "preserve document tree export file permissions",
                    source,
                ));
            }
        };

        if let Err(error) = write_encoded_replacement_file(&mut file, export) {
            drop(file);
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }

        drop(file);

        if let Err(source) = self.replace_file(&temp_path, file_path) {
            let _ = fs::remove_file(&temp_path);
            return Err(AppError::io_with_user_message(
                "replace document tree export file",
                IoUserMessage::WriteTextFile,
                source,
            ));
        }

        Ok(())
    }

    #[cfg(unix)]
    fn replace_file(&mut self, temp_path: &Path, file_path: &Path) -> io::Result<()> {
        fs::rename(temp_path, file_path)?;
        self.dirty_parent_directories
            .insert(parent_directory(file_path));
        Ok(())
    }

    #[cfg(not(unix))]
    fn replace_file(&mut self, temp_path: &Path, file_path: &Path) -> io::Result<()> {
        file_system::replace_file(temp_path, file_path)
    }

    fn finish(&mut self) -> Result<(), AppError> {
        self.finish_inner()
    }

    #[cfg(unix)]
    fn finish_inner(&mut self) -> Result<(), AppError> {
        for directory in &self.dirty_parent_directories {
            File::open(directory)
                .and_then(|file| file.sync_all())
                .map_err(|source| AppError::io("sync document tree export directory", source))?;
        }
        self.dirty_parent_directories.clear();
        Ok(())
    }

    #[cfg(not(unix))]
    fn finish_inner(&mut self) -> Result<(), AppError> {
        Ok(())
    }
}

fn finish_text_tree_export_write(
    result: Result<(), AppError>,
    write_session: &mut TextTreeExportWriteSession,
) -> Result<(), AppError> {
    let finish_result = write_session.finish();
    match (result, finish_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (_, Err(error)) => Err(error),
    }
}

#[cfg(unix)]
fn parent_directory(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn write_encoded_replacement_file(
    file: &mut File,
    export: &EncodedTextExport,
) -> Result<(), AppError> {
    write_encoded_text_export_to_file(file, export)
}

fn create_temp_replacement_file(path: &Path) -> Result<(PathBuf, File), AppError> {
    let file_name = path.file_name().ok_or_else(|| {
        AppError::io_with_user_message(
            "create temporary document tree export file",
            IoUserMessage::WriteTextFile,
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "document tree export path must include a file name",
            ),
        )
    })?;
    let unique = replacement_file_unique_part();

    for attempt in 0..REPLACEMENT_FILE_ATTEMPTS {
        let mut temp_file_name = OsString::from(".");
        temp_file_name.push(file_name);
        temp_file_name.push(format!(".j3treetext-export-{unique}-{attempt}.partial"));
        let temp_path = path.with_file_name(temp_file_name);

        match file_system::create_replacement_file(&temp_path, path) {
            Ok(file) => return Ok((temp_path, file)),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => {
                return Err(AppError::io_with_user_message(
                    "create temporary document tree export file",
                    IoUserMessage::WriteTextFile,
                    source,
                ));
            }
        }
    }

    Err(AppError::io_with_user_message(
        "create temporary document tree export file",
        IoUserMessage::WriteTextFile,
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique temporary document tree export file",
        ),
    ))
}

fn replacement_file_unique_part() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("{}-{timestamp}", std::process::id())
}

fn plan_document_tree_text_exports<'a>(
    destination: &Path,
    nodes: &'a [Node],
) -> Vec<PlannedTextExport<'a>> {
    let children_by_parent = sorted_children_by_parent(nodes);
    let mut exports = Vec::with_capacity(nodes.len());
    let mut pending = Vec::new();

    if let Some(roots) = children_by_parent.get(&None) {
        for root in roots.iter().rev() {
            pending.push((*root, destination.to_path_buf()));
        }
    }

    while let Some((node, directory)) = pending.pop() {
        let component = export_path_component(node);
        let file_path = directory.join(format!("{component}.txt"));
        exports.push(PlannedTextExport {
            node,
            directory: directory.clone(),
            file_path,
        });

        if let Some(children) = children_by_parent.get(&Some(node.id)) {
            let child_directory = directory.join(component);
            for child in children.iter().rev() {
                pending.push((*child, child_directory.clone()));
            }
        }
    }

    exports
}

fn export_content<'a>(
    node: &'a Node,
    content_overrides: &'a HashMap<i64, &'a str>,
    load_content: &mut impl FnMut(&'a Node) -> Result<Cow<'a, str>, AppError>,
) -> Result<Cow<'a, str>, AppError> {
    match content_overrides.get(&node.id) {
        Some(content) => Ok(Cow::Borrowed(*content)),
        None => load_content(node),
    }
}

fn sorted_children_by_parent(nodes: &[Node]) -> HashMap<Option<i64>, Vec<&Node>> {
    let mut children_by_parent: HashMap<Option<i64>, Vec<&Node>> =
        HashMap::with_capacity(nodes.len());
    for node in nodes {
        children_by_parent
            .entry(node.parent_id)
            .or_default()
            .push(node);
    }

    for children in children_by_parent.values_mut() {
        children.sort_by(compare_node_refs);
    }

    children_by_parent
}

fn compare_node_refs(left: &&Node, right: &&Node) -> Ordering {
    left.sort_order
        .cmp(&right.sort_order)
        .then_with(|| left.title.cmp(&right.title))
        .then_with(|| left.id.cmp(&right.id))
}

fn export_path_component(node: &Node) -> String {
    format!("{} - {}", node.id, sanitize_export_title(&node.title))
}

fn sanitize_export_title(title: &str) -> String {
    let mut safe = String::with_capacity(title.len().min(MAX_EXPORT_TITLE_CHARS));
    for (index, character) in title.chars().enumerate() {
        if index >= MAX_EXPORT_TITLE_CHARS {
            break;
        }

        if is_invalid_path_component_char(character) {
            safe.push('_');
        } else {
            safe.push(character);
        }
    }

    trim_trailing_windows_path_chars(&mut safe);
    if safe.trim().is_empty() {
        DEFAULT_EXPORT_TITLE.to_owned()
    } else {
        safe
    }
}

fn is_invalid_path_component_char(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
        )
}

fn trim_trailing_windows_path_chars(value: &mut String) {
    while matches!(value.chars().last(), Some(' ' | '.')) {
        value.pop();
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::domain::Document;

    #[test]
    fn exports_active_document_tree_as_txt_files() -> Result<(), Box<dyn Error>> {
        let destination = unique_test_dir("tree");
        let document = Document::new(vec![
            node(1, None, "Root", 0, "root body"),
            node(2, Some(1), "Alpha", 0, "alpha body"),
            node(3, Some(1), "Bad/Name", 1, "bad body"),
            node(4, Some(2), "Grand", 0, "grand body"),
        ])?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let count = write_document_tree_text_files(
                &destination,
                &document,
                TextEncoding::Utf8,
                &HashMap::new(),
            )?;

            assert_eq!(count, 4);
            assert_eq!(
                fs::read_to_string(destination.join("1 - Root.txt"))?,
                "root body"
            );
            assert_eq!(
                fs::read_to_string(destination.join("1 - Root").join("2 - Alpha.txt"))?,
                "alpha body"
            );
            assert_eq!(
                fs::read_to_string(
                    destination
                        .join("1 - Root")
                        .join("2 - Alpha")
                        .join("4 - Grand.txt")
                )?,
                "grand body"
            );
            assert_eq!(
                fs::read_to_string(destination.join("1 - Root").join("3 - Bad_Name.txt"))?,
                "bad body"
            );
            Ok(())
        })();
        let cleanup = remove_test_dir(&destination);

        result?;
        cleanup?;
        Ok(())
    }

    #[test]
    fn dirty_content_overrides_stored_content() -> Result<(), Box<dyn Error>> {
        let destination = unique_test_dir("override");
        let document = Document::new(vec![
            node(1, None, "Root", 0, ""),
            node(2, Some(1), "Draft", 0, "stored"),
        ])?;
        let mut overrides = HashMap::new();
        overrides.insert(2, "dirty draft");

        let result = (|| -> Result<(), Box<dyn Error>> {
            write_document_tree_text_files(
                &destination,
                &document,
                TextEncoding::Utf8,
                &overrides,
            )?;

            assert_eq!(
                fs::read_to_string(destination.join("1 - Root").join("2 - Draft.txt"))?,
                "dirty draft"
            );
            Ok(())
        })();
        let cleanup = remove_test_dir(&destination);

        result?;
        cleanup?;
        Ok(())
    }

    #[test]
    fn utf8_content_loader_runs_once_per_exported_node() -> Result<(), Box<dyn Error>> {
        let destination = unique_test_dir("loader-once");
        let document = Document::new(vec![
            node(1, None, "Root", 0, ""),
            node(2, Some(1), "Child", 0, ""),
        ])?;
        let mut loaded_node_ids = Vec::new();

        let result = (|| -> Result<(), Box<dyn Error>> {
            let count = write_document_tree_text_files_with_content_loader(
                &destination,
                &document,
                TextEncoding::Utf8,
                &HashMap::new(),
                |node| {
                    loaded_node_ids.push(node.id);
                    Ok(Cow::Owned(format!("loaded {}", node.id)))
                },
            )?;

            assert_eq!(count, 2);
            assert_eq!(loaded_node_ids, vec![1, 2]);
            assert_eq!(
                fs::read_to_string(destination.join("1 - Root.txt"))?,
                "loaded 1"
            );
            assert_eq!(
                fs::read_to_string(destination.join("1 - Root").join("2 - Child.txt"))?,
                "loaded 2"
            );
            Ok(())
        })();
        let cleanup = remove_test_dir(&destination);

        result?;
        cleanup?;
        Ok(())
    }

    #[test]
    fn legacy_encoded_content_loader_runs_once_per_exported_node() -> Result<(), Box<dyn Error>> {
        let destination = unique_test_dir("legacy-loader-once");
        let document = Document::new(vec![
            node(1, None, "Root", 0, ""),
            node(2, Some(1), "Child", 0, ""),
        ])?;
        let mut loaded_node_ids = Vec::new();

        let result = (|| -> Result<(), Box<dyn Error>> {
            let count = write_document_tree_text_files_with_content_loader(
                &destination,
                &document,
                TextEncoding::Windows1252,
                &HashMap::new(),
                |node| {
                    loaded_node_ids.push(node.id);
                    Ok(Cow::Owned(format!("loaded {}", node.id)))
                },
            )?;

            assert_eq!(count, 2);
            assert_eq!(loaded_node_ids, vec![1, 2]);
            assert_eq!(
                fs::read_to_string(destination.join("1 - Root.txt"))?,
                "loaded 1"
            );
            assert_eq!(
                fs::read_to_string(destination.join("1 - Root").join("2 - Child.txt"))?,
                "loaded 2"
            );
            Ok(())
        })();
        let cleanup = remove_test_dir(&destination);

        result?;
        cleanup?;
        Ok(())
    }

    #[test]
    fn export_replaces_matching_files_and_preserves_unrelated_files() -> Result<(), Box<dyn Error>>
    {
        let destination = unique_test_dir("replace");
        fs::create_dir(&destination)?;
        fs::write(destination.join("1 - Root.txt"), "old root")?;
        fs::write(destination.join("unrelated.txt"), "keep")?;
        let document = Document::new(vec![node(1, None, "Root", 0, "new root")])?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let count = write_document_tree_text_files(
                &destination,
                &document,
                TextEncoding::Utf8,
                &HashMap::new(),
            )?;

            assert_eq!(count, 1);
            assert_eq!(
                fs::read_to_string(destination.join("1 - Root.txt"))?,
                "new root"
            );
            assert_eq!(
                fs::read_to_string(destination.join("unrelated.txt"))?,
                "keep"
            );
            Ok(())
        })();
        let cleanup = remove_test_dir(&destination);

        result?;
        cleanup?;
        Ok(())
    }

    #[test]
    fn encoding_is_validated_before_creating_export_folder() -> Result<(), Box<dyn Error>> {
        let destination = unique_test_dir("prevalidate");
        let document = Document::new(vec![node(1, None, "Root", 0, "emoji 😀")])?;

        let error = write_document_tree_text_files(
            &destination,
            &document,
            TextEncoding::Windows1252,
            &HashMap::new(),
        )
        .expect_err("unrepresentable text should fail before writing");

        assert!(matches!(error, AppError::TextEncoding { .. }));
        assert!(!destination.exists());
        Ok(())
    }

    #[test]
    fn encoding_validation_prevents_partial_replacements() -> Result<(), Box<dyn Error>> {
        let destination = unique_test_dir("prevalidate-replace");
        fs::create_dir(&destination)?;
        fs::write(destination.join("1 - Root.txt"), "old root")?;
        let document = Document::new(vec![
            node(1, None, "Root", 0, "new root"),
            node(2, Some(1), "Child", 0, "emoji 😀"),
        ])?;

        let result = (|| -> Result<(), Box<dyn Error>> {
            let error = write_document_tree_text_files(
                &destination,
                &document,
                TextEncoding::Windows1252,
                &HashMap::new(),
            )
            .expect_err("unrepresentable child should fail before replacing files");

            assert!(matches!(error, AppError::TextEncoding { .. }));
            assert_eq!(
                fs::read_to_string(destination.join("1 - Root.txt"))?,
                "old root"
            );
            assert!(!destination.join("1 - Root").exists());
            Ok(())
        })();
        let cleanup = remove_test_dir(&destination);

        result?;
        cleanup?;
        Ok(())
    }

    fn node(id: i64, parent_id: Option<i64>, title: &str, sort_order: i64, content: &str) -> Node {
        Node {
            id,
            parent_id,
            title: title.to_owned(),
            sort_order,
            content: content.to_owned(),
            created_at: "2026-05-31T00:00:00Z".to_owned(),
            updated_at: "2026-05-31T00:00:00Z".to_owned(),
            deleted_at: None,
        }
    }

    fn unique_test_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        std::env::temp_dir().join(format!(
            "j3treetext-export-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn remove_test_dir(path: &Path) -> Result<(), Box<dyn Error>> {
        if path.exists() {
            fs::remove_dir_all(path)?;
        }
        Ok(())
    }
}
