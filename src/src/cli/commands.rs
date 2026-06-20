use crate::domain::{Node, SiblingMoveDirection, SEARCH_RESULT_LIMIT};
use crate::error::AppError;
use crate::infra::sqlite::SqliteDocumentRepository;

use super::args::CliArgs;
use super::content::{
    read_content_source, reject_combined_content_byte_len_over_limit,
    reject_content_over_byte_limit,
};
use super::output;
use super::parser::{
    parse_create_args, parse_edit_args, parse_move_args, parse_node_id_args, parse_rename_args,
    parse_search_args, parse_show_args,
};

pub(super) fn dispatch(
    command: &str,
    repository: &mut SqliteDocumentRepository,
    args: CliArgs,
) -> Result<(), AppError> {
    match command {
        "tree" => run_tree(repository, args),
        "show" => run_show(repository, args),
        "create" => run_create(repository, args),
        "edit" => run_edit(repository, args),
        "rename" => run_rename(repository, args),
        "search" => run_search(repository, args),
        "delete" => run_delete(repository, args),
        "trash" => run_trash(repository, args),
        "restore" => run_restore(repository, args),
        "purge" => run_purge(repository, args),
        "move" => run_move(repository, args),
        "move-up" => run_move_within_parent(repository, args, SiblingMoveDirection::Up),
        "move-down" => run_move_within_parent(repository, args, SiblingMoveDirection::Down),
        _ => Err(AppError::user(format!(
            "알 수 없는 CLI 명령입니다: {command}\n사용법을 보려면 --help를 실행하세요."
        ))),
    }
}

fn run_tree(repository: &SqliteDocumentRepository, args: CliArgs) -> Result<(), AppError> {
    args.ensure_empty("tree")?;
    let document = repository.load_document_metadata()?;
    output::print_active_tree(document.nodes())
}

fn run_show(repository: &SqliteDocumentRepository, args: CliArgs) -> Result<(), AppError> {
    let options = parse_show_args(args)?;
    let node = if options.trash_only {
        find_deleted_node(repository, options.node_id)?
    } else {
        resolve_show_node(find_active_node(repository, options.node_id)?, || {
            find_deleted_node(repository, options.node_id)
        })?
    };

    output::print_node(&node)
}

fn resolve_show_node(
    active_node: Option<Node>,
    find_deleted_node: impl FnOnce() -> Result<Node, AppError>,
) -> Result<Node, AppError> {
    match active_node {
        Some(node) => Ok(node),
        None => find_deleted_node(),
    }
}

fn run_create(repository: &mut SqliteDocumentRepository, args: CliArgs) -> Result<(), AppError> {
    let command = parse_create_args(args)?;
    let content = read_content_source(command.content)?;
    reject_content_over_byte_limit(&content)?;
    let node = if content.is_empty() {
        repository.create_child_node(command.parent_id, &command.title)?
    } else {
        repository.create_document_with_content(command.parent_id, &command.title, &content)?
    };

    output::print_created(&node)
}

fn run_edit(repository: &mut SqliteDocumentRepository, args: CliArgs) -> Result<(), AppError> {
    let command = parse_edit_args(args)?;
    let updated_at = if command.append {
        let (content_byte_len, expected_updated_at) =
            repository.load_active_node_content_byte_len(command.node_id)?;
        let input = read_content_source(command.content)?;
        reject_combined_content_byte_len_over_limit(content_byte_len, input.len() as u64)?;
        repository.append_document_content_with_known_valid_existing_content(
            command.node_id,
            &input,
            &expected_updated_at,
        )?
    } else {
        let expected_updated_at = repository.load_active_node_updated_at(command.node_id)?;
        let content = read_content_source(command.content)?;
        reject_content_over_byte_limit(&content)?;
        repository.update_document_content(command.node_id, &content, &expected_updated_at)?
    };

    output::print_updated(command.node_id, &updated_at)
}

fn run_rename(repository: &mut SqliteDocumentRepository, args: CliArgs) -> Result<(), AppError> {
    let command = parse_rename_args(args)?;
    repository.rename_node(command.node_id, &command.title)?;
    output::print_renamed(command.node_id, &command.title)
}

fn run_search(repository: &SqliteDocumentRepository, args: CliArgs) -> Result<(), AppError> {
    let query = parse_search_args(args)?;
    let results = repository.search_documents(&query, SEARCH_RESULT_LIMIT)?;
    output::print_search_results(&results)
}

fn run_delete(repository: &mut SqliteDocumentRepository, args: CliArgs) -> Result<(), AppError> {
    let node_id = parse_node_id_args(args, "delete")?;
    repository.soft_delete_node_cascade(node_id)?;
    output::print_deleted(node_id)
}

fn run_trash(repository: &SqliteDocumentRepository, args: CliArgs) -> Result<(), AppError> {
    args.ensure_empty("trash")?;
    let nodes = repository.load_deleted_nodes()?;
    output::print_trash_tree(&nodes)
}

fn run_restore(repository: &mut SqliteDocumentRepository, args: CliArgs) -> Result<(), AppError> {
    let node_id = parse_node_id_args(args, "restore")?;
    repository.restore_deleted_node_cascade(node_id)?;
    output::print_restored(node_id)
}

fn run_purge(repository: &mut SqliteDocumentRepository, args: CliArgs) -> Result<(), AppError> {
    let node_id = parse_node_id_args(args, "purge")?;
    repository.permanently_delete_node_cascade(node_id)?;
    output::print_purged(node_id)
}

fn run_move(repository: &mut SqliteDocumentRepository, args: CliArgs) -> Result<(), AppError> {
    let command = parse_move_args(args)?;
    repository.move_node_to_parent_end(command.node_id, command.parent_id)?;
    output::print_moved_to_parent(command.node_id, command.parent_id)
}

fn run_move_within_parent(
    repository: &mut SqliteDocumentRepository,
    args: CliArgs,
    direction: SiblingMoveDirection,
) -> Result<(), AppError> {
    let node_id = parse_node_id_args(args, "move-up/move-down")?;
    repository.move_node_within_parent(node_id, direction)?;
    output::print_moved(node_id)
}

fn find_active_node(
    repository: &SqliteDocumentRepository,
    node_id: i64,
) -> Result<Option<Node>, AppError> {
    repository.load_active_node(node_id)
}

fn find_deleted_node(
    repository: &SqliteDocumentRepository,
    node_id: i64,
) -> Result<Node, AppError> {
    repository.load_deleted_node(node_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_node_resolution_preserves_deleted_lookup_errors() {
        let error = match resolve_show_node(None, || Err(AppError::user("deleted lookup failed"))) {
            Ok(_) => panic!("deleted lookup error should be returned"),
            Err(error) => error,
        };

        assert_eq!(error.user_message(), "deleted lookup failed");
    }

    #[test]
    fn show_node_resolution_prefers_active_node() {
        let active = node(1, None, "Active", 0, None);

        let selected = resolve_show_node(Some(active), || {
            Err(AppError::user("deleted lookup should not run"))
        })
        .expect("active node should be returned");

        assert_eq!(selected.id, 1);
    }

    fn node(
        id: i64,
        parent_id: Option<i64>,
        title: &str,
        sort_order: i64,
        deleted_at: Option<&str>,
    ) -> Node {
        Node {
            id,
            parent_id,
            title: title.to_owned(),
            sort_order,
            content: String::new(),
            created_at: "2026-05-01T00:00:00Z".to_owned(),
            updated_at: "2026-05-01T00:00:00Z".to_owned(),
            deleted_at: deleted_at.map(str::to_owned),
        }
    }
}
