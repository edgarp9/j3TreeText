use std::{
    borrow::Borrow,
    collections::{HashMap, HashSet},
    hash::Hash,
};

use rusqlite::{params, params_from_iter, Transaction};

use crate::domain::DomainError;
use crate::error::AppError;

const DEFAULT_TITLE_CANDIDATE_LIMIT: i64 = 10_000;
const ACTIVE_CHILD_TITLE_PARENT_BATCH_SIZE: usize = 900;

pub(in crate::infra::sqlite) fn normalize_title_input(title: &str) -> Result<String, AppError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(DomainError::EmptyTitleInput.into());
    }

    if title.contains('\0') {
        return Err(DomainError::EmbeddedNulTitleInput.into());
    }

    Ok(title.to_owned())
}

pub(in crate::infra::sqlite) fn unique_child_title(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    base_title: &str,
) -> Result<String, AppError> {
    let existing_titles = sibling_candidate_titles(transaction, parent_id, base_title)?;

    if !existing_titles.contains(base_title) {
        return Ok(base_title.to_owned());
    }

    for suffix in 2..=DEFAULT_TITLE_CANDIDATE_LIMIT {
        let candidate = format!("{base_title} {suffix}");
        if !existing_titles.contains(&candidate) {
            return Ok(candidate);
        }
    }

    Err(AppError::internal_consistency(
        "choose unique node title",
        "could not find an available default title",
    ))
}

pub(in crate::infra::sqlite) fn sibling_candidate_titles(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    base_title: &str,
) -> Result<HashSet<String>, AppError> {
    let prefix_pattern = like_prefix_pattern(&format!("{base_title} "));
    let mut statement = transaction
        .prepare(
            "
        SELECT title
        FROM nodes
        WHERE deleted_at IS NULL
            AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
            AND (title = ?2 OR title LIKE ?3 ESCAPE '^')
        ",
        )
        .map_err(|source| AppError::sqlite("prepare SQLite sibling title query", source))?;

    let rows = statement
        .query_map(params![parent_id, base_title, &prefix_pattern], |row| {
            row.get(0)
        })
        .map_err(|source| AppError::sqlite("query SQLite sibling titles", source))?;

    let mut titles = HashSet::new();
    for row in rows {
        titles.insert(
            row.map_err(|source| AppError::sqlite("read SQLite sibling title row", source))?,
        );
    }

    Ok(titles)
}

pub(in crate::infra::sqlite) fn active_child_titles_by_parent<I>(
    transaction: &Transaction<'_>,
    parent_ids: I,
) -> Result<HashMap<Option<i64>, HashSet<String>>, AppError>
where
    I: IntoIterator<Item = Option<i64>>,
{
    let mut include_root_parent = false;
    let mut parent_id_set = HashSet::new();
    for parent_id in parent_ids {
        match parent_id {
            Some(parent_id) => {
                parent_id_set.insert(parent_id);
            }
            None => include_root_parent = true,
        }
    }
    let mut parent_ids = parent_id_set.into_iter().collect::<Vec<_>>();
    parent_ids.sort_unstable();

    let mut titles_by_parent = HashMap::new();
    if include_root_parent {
        titles_by_parent.insert(None, HashSet::new());
    }
    for parent_id in &parent_ids {
        titles_by_parent.insert(Some(*parent_id), HashSet::new());
    }

    if parent_ids.is_empty() {
        if include_root_parent {
            load_active_child_titles_for_parent_filter(
                transaction,
                true,
                &[],
                &mut titles_by_parent,
            )?;
        }
        return Ok(titles_by_parent);
    }

    let mut include_root_parent_in_batch = include_root_parent;
    for parent_id_batch in parent_ids.chunks(ACTIVE_CHILD_TITLE_PARENT_BATCH_SIZE) {
        load_active_child_titles_for_parent_filter(
            transaction,
            include_root_parent_in_batch,
            parent_id_batch,
            &mut titles_by_parent,
        )?;
        include_root_parent_in_batch = false;
    }

    Ok(titles_by_parent)
}

fn load_active_child_titles_for_parent_filter(
    transaction: &Transaction<'_>,
    include_root_parent: bool,
    parent_ids: &[i64],
    titles_by_parent: &mut HashMap<Option<i64>, HashSet<String>>,
) -> Result<(), AppError> {
    if parent_ids.is_empty() && !include_root_parent {
        return Ok(());
    }

    let mut parent_filters = Vec::new();
    if include_root_parent {
        parent_filters.push("parent_id IS NULL".to_owned());
    }
    if !parent_ids.is_empty() {
        let placeholders = vec!["?"; parent_ids.len()].join(", ");
        parent_filters.push(format!("parent_id IN ({placeholders})"));
    }

    let sql = format!(
        "
        SELECT parent_id, title
        FROM nodes
        WHERE deleted_at IS NULL
            AND ({})
        ",
        parent_filters.join(" OR ")
    );
    let mut statement = transaction
        .prepare(&sql)
        .map_err(|source| AppError::sqlite("prepare SQLite active child title query", source))?;
    let rows = statement
        .query_map(params_from_iter(parent_ids.iter()), |row| {
            Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|source| AppError::sqlite("query SQLite active child titles", source))?;

    for row in rows {
        let (parent_id, title) =
            row.map_err(|source| AppError::sqlite("read SQLite active child title row", source))?;
        titles_by_parent.entry(parent_id).or_default().insert(title);
    }

    Ok(())
}

pub(in crate::infra::sqlite) fn like_prefix_pattern(prefix: &str) -> String {
    let mut pattern = String::with_capacity(prefix.len() + 1);
    for character in prefix.chars() {
        if matches!(character, '%' | '_' | '^') {
            pattern.push('^');
        }
        pattern.push(character);
    }
    pattern.push('%');
    pattern
}

pub(in crate::infra::sqlite) fn unique_restored_child_title(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    base_title: &str,
) -> Result<String, AppError> {
    unique_restored_child_title_with_reserved(transaction, parent_id, base_title, &[])
}

pub(in crate::infra::sqlite) fn unique_restored_child_title_with_reserved(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    base_title: &str,
    reserved_titles: &[String],
) -> Result<String, AppError> {
    let mut active_titles_by_parent =
        active_child_titles_by_parent(transaction, std::iter::once(parent_id))?;
    let active_titles = active_titles_by_parent.entry(parent_id).or_default();
    let reserved_titles = reserved_titles
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();

    unique_restored_child_title_from_titles(active_titles, base_title, &reserved_titles)
}

pub(in crate::infra::sqlite) fn unique_restored_child_title_from_titles<R>(
    active_titles: &HashSet<String>,
    base_title: &str,
    reserved_titles: &HashSet<R>,
) -> Result<String, AppError>
where
    R: Borrow<str> + Eq + Hash,
{
    if restored_child_title_available(active_titles, base_title, reserved_titles) {
        return Ok(base_title.to_owned());
    }

    let restored_base = format!("{base_title} (restored)");
    if restored_child_title_available(active_titles, &restored_base, reserved_titles) {
        return Ok(restored_base);
    }

    for suffix in 2..=DEFAULT_TITLE_CANDIDATE_LIMIT {
        let candidate = format!("{restored_base} {suffix}");
        if restored_child_title_available(active_titles, &candidate, reserved_titles) {
            return Ok(candidate);
        }
    }

    Err(AppError::internal_consistency(
        "choose restored node title",
        "could not find an available restored title",
    ))
}

pub(in crate::infra::sqlite) fn restored_child_title_available<R>(
    active_titles: &HashSet<String>,
    title: &str,
    reserved_titles: &HashSet<R>,
) -> bool
where
    R: Borrow<str> + Eq + Hash,
{
    if reserved_titles.contains(title) {
        return false;
    }

    !active_titles.contains(title)
}

pub(in crate::infra::sqlite) fn ensure_unique_child_title(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    title: &str,
    excluded_node_id: Option<i64>,
) -> Result<(), AppError> {
    if child_title_exists(transaction, parent_id, title, excluded_node_id)? {
        return Err(DomainError::DuplicateSiblingTitle {
            parent_id,
            title: title.to_owned(),
        }
        .into());
    }

    Ok(())
}

pub(in crate::infra::sqlite) fn child_title_exists(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
    title: &str,
    excluded_node_id: Option<i64>,
) -> Result<bool, AppError> {
    let count: i64 = transaction
        .query_row(
            "
        SELECT COUNT(*)
        FROM nodes
        WHERE deleted_at IS NULL
            AND title = ?2
            AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
            AND (?3 IS NULL OR id <> ?3)
        ",
            params![parent_id, title, excluded_node_id],
            |row| row.get(0),
        )
        .map_err(|source| AppError::sqlite("check SQLite sibling node title", source))?;

    Ok(count > 0)
}
