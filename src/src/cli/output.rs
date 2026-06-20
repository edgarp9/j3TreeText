use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};

use crate::domain::{DocumentSearchResult, Node};
use crate::error::AppError;

const HELP_TEXT: &str = "\
j3TreeTextCli - SQLite document tree CLI

Usage:
  j3TreeTextCli [--db <path>] <command> [args]
  j3TreeTextCli --help

Commands:
  tree
      Print the active document tree.
  show [--trash] <node_id>
      Print one document and its content. Without --trash, active nodes are checked first.
  create --parent <node_id> --title <title> [--content <text> | --content-file <path> | --stdin]
      Create a child document.
  edit <node_id> (--content <text> | --content-file <path> | --stdin) [--append]
      Replace or append document content.
  rename <node_id> --title <title>
      Rename an active document.
  search <query>
      Search active document titles and content.
  delete <node_id>
      Soft-delete a document subtree.
  trash
      Print deleted documents.
  restore <node_id>
      Restore a deleted document subtree.
  purge <node_id>
      Permanently delete a deleted document subtree.
  move <node_id> --parent <node_id>
      Move a document to the end of another document's children.
  move-up <node_id>
      Move a document one position up within its parent.
  move-down <node_id>
      Move a document one position down within its parent.
";

const MAX_TREE_RENDER_DEPTH: usize = 1024;
const TREE_INDENT: &str = "  ";

pub(super) fn print_help() -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    write!(stdout, "{HELP_TEXT}").map_err(stdout_write_error)
}

pub(super) fn print_active_tree(nodes: &[Node]) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    write_tree_preserving_input_order(&mut stdout, nodes, TreeMode::Active)
}

pub(super) fn print_trash_tree(nodes: &[Node]) -> Result<(), AppError> {
    print_tree(nodes, TreeMode::Trash)
}

pub(super) fn print_node(node: &Node) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "id: {}", node.id).map_err(stdout_write_error)?;
    writeln!(
        stdout,
        "parent_id: {}",
        node.parent_id
            .map(|parent_id| parent_id.to_string())
            .unwrap_or_else(|| "-".to_owned())
    )
    .map_err(stdout_write_error)?;
    writeln!(stdout, "title: {}", node.title).map_err(stdout_write_error)?;
    writeln!(stdout, "created_at: {}", node.created_at).map_err(stdout_write_error)?;
    writeln!(stdout, "updated_at: {}", node.updated_at).map_err(stdout_write_error)?;
    if let Some(deleted_at) = &node.deleted_at {
        writeln!(stdout, "deleted_at: {deleted_at}").map_err(stdout_write_error)?;
    }
    writeln!(stdout, "content:").map_err(stdout_write_error)?;
    write!(stdout, "{}", node.content).map_err(stdout_write_error)?;
    if !node.content.ends_with('\n') {
        writeln!(stdout).map_err(stdout_write_error)?;
    }
    Ok(())
}

pub(super) fn print_search_results(results: &[DocumentSearchResult]) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    for result in results {
        let parent = result.parent_title.as_deref().unwrap_or("-");
        writeln!(
            stdout,
            "[{}] {} (parent: {parent})",
            result.node.id, result.node.title
        )
        .map_err(stdout_write_error)?;
    }
    Ok(())
}

pub(super) fn print_created(node: &Node) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "created [{}] {}", node.id, node.title).map_err(stdout_write_error)
}

pub(super) fn print_updated(node_id: i64, updated_at: &str) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "updated [{node_id}] {updated_at}").map_err(stdout_write_error)
}

pub(super) fn print_renamed(node_id: i64, title: &str) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "renamed [{node_id}] {title}").map_err(stdout_write_error)
}

pub(super) fn print_deleted(node_id: i64) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "deleted [{node_id}]").map_err(stdout_write_error)
}

pub(super) fn print_restored(node_id: i64) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "restored [{node_id}]").map_err(stdout_write_error)
}

pub(super) fn print_purged(node_id: i64) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "purged [{node_id}]").map_err(stdout_write_error)
}

pub(super) fn print_moved_to_parent(node_id: i64, parent_id: i64) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "moved [{node_id}] to parent [{parent_id}]").map_err(stdout_write_error)
}

pub(super) fn print_moved(node_id: i64) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "moved [{node_id}]").map_err(stdout_write_error)
}

fn print_tree(nodes: &[Node], mode: TreeMode) -> Result<(), AppError> {
    let mut stdout = io::stdout().lock();
    write_tree(&mut stdout, nodes, mode)
}

fn stdout_write_error(source: io::Error) -> AppError {
    AppError::io("write stdout", source)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeMode {
    Active,
    Trash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SiblingOrder {
    Sorted,
    Input,
}

fn write_tree<W: Write>(writer: &mut W, nodes: &[Node], mode: TreeMode) -> Result<(), AppError> {
    write_tree_with_order(writer, nodes, mode, SiblingOrder::Sorted)
}

fn write_tree_preserving_input_order<W: Write>(
    writer: &mut W,
    nodes: &[Node],
    mode: TreeMode,
) -> Result<(), AppError> {
    write_tree_with_order(writer, nodes, mode, SiblingOrder::Input)
}

fn write_tree_with_order<W: Write>(
    writer: &mut W,
    nodes: &[Node],
    mode: TreeMode,
    sibling_order: SiblingOrder,
) -> Result<(), AppError> {
    let mut children_by_parent: HashMap<Option<i64>, Vec<&Node>> = HashMap::new();
    for node in nodes {
        children_by_parent
            .entry(node.parent_id)
            .or_default()
            .push(node);
    }
    if sibling_order == SiblingOrder::Sorted {
        for children in children_by_parent.values_mut() {
            children.sort_by(compare_node_refs);
        }
    }

    let mut visited = HashSet::new();

    match mode {
        TreeMode::Active => {
            if let Some(roots) = children_by_parent.get(&None) {
                for root in roots {
                    write_tree_lines(writer, root, mode, &children_by_parent, &mut visited)?;
                }
            }
        }
        TreeMode::Trash => {
            for root in trash_roots(nodes) {
                write_tree_lines(writer, root, mode, &children_by_parent, &mut visited)?;
            }
        }
    }

    Ok(())
}

fn trash_roots(nodes: &[Node]) -> Vec<&Node> {
    let deleted_ids: HashSet<i64> = nodes.iter().map(|node| node.id).collect();
    let mut roots: Vec<&Node> = nodes
        .iter()
        .filter(|node| {
            node.parent_id
                .map(|parent_id| !deleted_ids.contains(&parent_id))
                .unwrap_or(true)
        })
        .collect();
    roots.sort_by(compare_node_refs);
    roots
}

fn write_tree_lines<W: Write>(
    writer: &mut W,
    root: &Node,
    mode: TreeMode,
    children_by_parent: &HashMap<Option<i64>, Vec<&Node>>,
    visited: &mut HashSet<i64>,
) -> Result<(), AppError> {
    let mut pending = vec![(root, 0usize)];
    while let Some((node, depth)) = pending.pop() {
        if !visited.insert(node.id) {
            continue;
        }

        write_tree_line(writer, node, depth, mode)?;

        if let Some(children) = children_by_parent.get(&Some(node.id)) {
            let child_depth = depth.checked_add(1).ok_or_else(tree_depth_error)?;
            for child in children.iter().rev() {
                pending.push((*child, child_depth));
            }
        }
    }

    Ok(())
}

fn write_tree_line<W: Write>(
    writer: &mut W,
    node: &Node,
    depth: usize,
    mode: TreeMode,
) -> Result<(), AppError> {
    if depth > MAX_TREE_RENDER_DEPTH {
        return Err(tree_depth_error());
    }

    for _ in 0..depth {
        write!(writer, "{TREE_INDENT}").map_err(stdout_write_error)?;
    }

    write!(writer, "- [{}] {}", node.id, node.title).map_err(stdout_write_error)?;
    if mode == TreeMode::Trash {
        if let Some(deleted_at) = &node.deleted_at {
            write!(writer, " (deleted: {deleted_at})").map_err(stdout_write_error)?;
        }
    }
    writeln!(writer).map_err(stdout_write_error)?;

    Ok(())
}

fn tree_depth_error() -> AppError {
    AppError::user(format!(
        "문서 트리가 너무 깊어 CLI에서 출력할 수 없습니다. 최대 출력 깊이: {MAX_TREE_RENDER_DEPTH}"
    ))
}

fn compare_node_refs(left: &&Node, right: &&Node) -> Ordering {
    left.sort_order
        .cmp(&right.sort_order)
        .then_with(|| left.title.cmp(&right.title))
        .then_with(|| left.id.cmp(&right.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_tree_lines_are_indented_by_parent() {
        let nodes = vec![
            node(1, None, "Root", 0, None),
            node(2, Some(1), "Alpha", 0, None),
            node(3, Some(2), "Child", 0, None),
            node(4, Some(1), "Beta", 1, None),
        ];

        assert_eq!(
            render_tree(&nodes, TreeMode::Active).expect("active tree should render"),
            "- [1] Root\n  - [2] Alpha\n    - [3] Child\n  - [4] Beta\n"
        );
    }

    #[test]
    fn active_tree_sorts_arbitrary_sibling_input() {
        let nodes = vec![
            node(1, None, "Root", 0, None),
            node(3, Some(1), "Beta", 1, None),
            node(2, Some(1), "Alpha", 0, None),
        ];

        assert_eq!(
            render_tree(&nodes, TreeMode::Active).expect("active tree should render"),
            "- [1] Root\n  - [2] Alpha\n  - [3] Beta\n"
        );
    }

    #[test]
    fn active_tree_can_preserve_trusted_display_order() {
        let nodes = vec![
            node(1, None, "Root", 0, None),
            node(3, Some(1), "Beta", 1, None),
            node(2, Some(1), "Alpha", 0, None),
        ];

        assert_eq!(
            render_tree_preserving_input_order(&nodes, TreeMode::Active)
                .expect("active tree should render"),
            "- [1] Root\n  - [3] Beta\n  - [2] Alpha\n"
        );
    }

    #[test]
    fn trash_tree_keeps_deleted_parent_hierarchy() {
        let nodes = vec![
            node(10, Some(1), "Deleted Parent", 0, Some("deleted")),
            node(11, Some(10), "Deleted Child", 0, Some("deleted")),
            node(12, Some(1), "Deleted Sibling", 1, Some("deleted")),
        ];

        assert_eq!(
            render_tree(&nodes, TreeMode::Trash).expect("trash tree should render"),
            "- [10] Deleted Parent (deleted: deleted)\n  - [11] Deleted Child (deleted: deleted)\n- [12] Deleted Sibling (deleted: deleted)\n"
        );
    }

    #[test]
    fn active_tree_lines_reject_depth_beyond_render_limit() {
        let nodes = deep_tree_nodes(MAX_TREE_RENDER_DEPTH + 2, None);
        let mut output = Vec::new();

        let error = write_tree(&mut output, &nodes, TreeMode::Active)
            .expect_err("overly deep active tree should be rejected");

        assert_eq!(
            error.user_message(),
            "문서 트리가 너무 깊어 CLI에서 출력할 수 없습니다. 최대 출력 깊이: 1024"
        );
    }

    #[test]
    fn trash_tree_lines_reject_depth_beyond_render_limit() {
        let nodes = deep_tree_nodes(MAX_TREE_RENDER_DEPTH + 2, Some("deleted"));
        let mut output = Vec::new();

        let error = write_tree(&mut output, &nodes, TreeMode::Trash)
            .expect_err("overly deep trash tree should be rejected");

        assert_eq!(
            error.user_message(),
            "문서 트리가 너무 깊어 CLI에서 출력할 수 없습니다. 최대 출력 깊이: 1024"
        );
    }

    fn render_tree(nodes: &[Node], mode: TreeMode) -> Result<String, AppError> {
        let mut output = Vec::new();
        write_tree(&mut output, nodes, mode)?;
        Ok(String::from_utf8(output).expect("tree output should be valid utf-8"))
    }

    fn render_tree_preserving_input_order(
        nodes: &[Node],
        mode: TreeMode,
    ) -> Result<String, AppError> {
        let mut output = Vec::new();
        write_tree_preserving_input_order(&mut output, nodes, mode)?;
        Ok(String::from_utf8(output).expect("tree output should be valid utf-8"))
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

    fn deep_tree_nodes(count: usize, deleted_at: Option<&str>) -> Vec<Node> {
        (0..count)
            .map(|index| {
                let id = (index + 1) as i64;
                let parent_id = if index == 0 { None } else { Some(index as i64) };
                node(id, parent_id, "Node", 0, deleted_at)
            })
            .collect()
    }
}
