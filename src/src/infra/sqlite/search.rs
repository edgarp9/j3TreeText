use rusqlite::{params, Connection};

use crate::domain::DocumentSearchResult;
use crate::error::AppError;

use super::node::{node_from_record, NodeRecord};
use super::SqliteDocumentRepository;

const MIN_FTS_TRIGRAM_CHARS: usize = 3;
const SHORT_QUERY_CONTENT_SCAN_MIN_ROWS: i64 = 512;
const SHORT_QUERY_CONTENT_SCAN_MAX_ROWS: i64 = 4096;
const SHORT_QUERY_CONTENT_SCAN_LIMIT_MULTIPLIER: i64 = 32;

impl SqliteDocumentRepository {
    pub fn search_documents(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<DocumentSearchResult>, AppError> {
        query_documents(&self.connection, query, limit)
    }
}

fn query_documents(
    connection: &Connection,
    query: &str,
    limit: usize,
) -> Result<Vec<DocumentSearchResult>, AppError> {
    if limit == 0 {
        return Ok(Vec::new());
    }

    let Some(pattern) = like_contains_pattern(query) else {
        return Ok(Vec::new());
    };
    let title_fts_query = fts_trigram_title_query(query);
    let content_fts_query = fts_trigram_content_query(query);
    let is_short_query = is_short_trigram_query(query);
    let sql_limit = i64::try_from(limit)
        .map_err(|_| AppError::platform("search SQLite documents", "search limit is invalid"))?;
    let uses_fts = title_fts_query.is_some() && content_fts_query.is_some();

    if is_short_query && !uses_fts {
        return query_short_documents(connection, &pattern, sql_limit);
    }

    if let (Some(title_fts_query), Some(content_fts_query)) = (title_fts_query, content_fts_query) {
        return query_fts_documents(
            connection,
            &pattern,
            &title_fts_query,
            &content_fts_query,
            sql_limit,
        );
    }

    let mut statement = connection
        .prepare_cached(
            "
            WITH matching_documents(rowid, title_matched, content_matched) AS (
                SELECT
                    id,
                    title LIKE ?1 ESCAPE '^',
                    content LIKE ?1 ESCAPE '^'
                FROM nodes
                WHERE deleted_at IS NULL
                    AND (
                        title LIKE ?1 ESCAPE '^'
                        OR content LIKE ?1 ESCAPE '^'
                    )
            )
            SELECT
                document.id,
                document.parent_id,
                document.title,
                document.sort_order,
                '',
                document.created_at,
                document.updated_at,
                document.deleted_at,
                parent.title,
                matching_documents.content_matched
            FROM matching_documents
            JOIN nodes document
                ON document.id = matching_documents.rowid
            LEFT JOIN nodes parent
                ON parent.id = document.parent_id
                AND parent.deleted_at IS NULL
            ORDER BY matching_documents.title_matched DESC, document.title COLLATE NOCASE, document.id
            LIMIT ?2
            ",
        )
        .map_err(|source| AppError::sqlite("prepare document search query", source))?;

    collect_search_results(&mut statement, params![&pattern, sql_limit])
}

fn query_fts_documents(
    connection: &Connection,
    pattern: &str,
    title_fts_query: &str,
    content_fts_query: &str,
    limit: i64,
) -> Result<Vec<DocumentSearchResult>, AppError> {
    let mut statement = connection
        .prepare_cached(
            "
        WITH title_matches(rowid) AS (
            SELECT node_search_fts.rowid
            FROM node_search_fts
            JOIN nodes document
                ON document.id = node_search_fts.rowid
            WHERE node_search_fts MATCH ?2
                AND document.deleted_at IS NULL
                AND document.title LIKE ?1 ESCAPE '^'
        ),
        content_matches(rowid) AS (
            SELECT node_search_fts.rowid
            FROM node_search_fts
            JOIN nodes document
                ON document.id = node_search_fts.rowid
            WHERE node_search_fts MATCH ?3
                AND document.deleted_at IS NULL
        ),
        matching_documents(rowid, title_matched, content_matched) AS (
            SELECT
                title_matches.rowid,
                1,
                content_matches.rowid IS NOT NULL
            FROM title_matches
            LEFT JOIN content_matches
                ON content_matches.rowid = title_matches.rowid

            UNION ALL

            SELECT
                content_matches.rowid,
                0,
                1
            FROM content_matches
            LEFT JOIN title_matches
                ON title_matches.rowid = content_matches.rowid
            WHERE title_matches.rowid IS NULL
        )
        SELECT
            document.id,
            document.parent_id,
            document.title,
            document.sort_order,
            '',
            document.created_at,
            document.updated_at,
            document.deleted_at,
            parent.title,
            matching_documents.content_matched
        FROM matching_documents
        JOIN nodes document
            ON document.id = matching_documents.rowid
        LEFT JOIN nodes parent
            ON parent.id = document.parent_id
            AND parent.deleted_at IS NULL
        ORDER BY matching_documents.title_matched DESC, document.title COLLATE NOCASE, document.id
        LIMIT ?4
        ",
        )
        .map_err(|source| AppError::sqlite("prepare FTS document search query", source))?;

    collect_search_results(
        &mut statement,
        params![pattern, title_fts_query, content_fts_query, limit],
    )
}

fn query_short_documents(
    connection: &Connection,
    pattern: &str,
    limit: i64,
) -> Result<Vec<DocumentSearchResult>, AppError> {
    let content_scan_limit = short_query_content_scan_limit(limit);
    let mut statement = connection
        .prepare_cached(
            "
            WITH title_matches(rowid, match_rank) AS (
                SELECT
                    id,
                    0
                FROM nodes
                WHERE deleted_at IS NULL
                    AND title LIKE ?1 ESCAPE '^'
                ORDER BY title COLLATE NOCASE, id
                LIMIT ?2
            ),
            remaining_limit(value) AS (
                SELECT
                    CASE
                        WHEN ?2 > COUNT(*) THEN ?2 - COUNT(*)
                        ELSE 0
                    END
                FROM title_matches
            ),
            content_candidates(rowid, title) AS (
                SELECT
                    document.id,
                    document.title
                FROM nodes document
                WHERE document.deleted_at IS NULL
                    AND (SELECT value FROM remaining_limit) > 0
                    AND NOT EXISTS (
                        SELECT 1
                        FROM title_matches
                        WHERE title_matches.rowid = document.id
                    )
                ORDER BY document.title COLLATE NOCASE, document.id
                LIMIT ?3
            ),
            content_matches(rowid, match_rank) AS (
                SELECT
                    document.id,
                    1
                FROM content_candidates
                JOIN nodes document
                    ON document.id = content_candidates.rowid
                WHERE document.content LIKE ?1 ESCAPE '^'
                ORDER BY content_candidates.title COLLATE NOCASE, content_candidates.rowid
                LIMIT (SELECT value FROM remaining_limit)
            ),
            matching_documents(rowid, match_rank, content_matched) AS (
                SELECT
                    rowid,
                    match_rank,
                    0
                FROM title_matches

                UNION ALL

                SELECT
                    rowid,
                    match_rank,
                    1
                FROM content_matches
            )
            SELECT
                document.id,
                document.parent_id,
                document.title,
                document.sort_order,
                '',
                document.created_at,
                document.updated_at,
                document.deleted_at,
                parent.title,
                matching_documents.content_matched
            FROM matching_documents
            JOIN nodes document
                ON document.id = matching_documents.rowid
            LEFT JOIN nodes parent
                ON parent.id = document.parent_id
                AND parent.deleted_at IS NULL
            ORDER BY matching_documents.match_rank, document.title COLLATE NOCASE, document.id
            ",
        )
        .map_err(|source| AppError::sqlite("prepare short document search query", source))?;

    collect_search_results(&mut statement, params![pattern, limit, content_scan_limit])
}

fn short_query_content_scan_limit(limit: i64) -> i64 {
    // Short queries cannot use the trigram index, so keep the content LIKE fallback bounded.
    let scaled_limit = limit.saturating_mul(SHORT_QUERY_CONTENT_SCAN_LIMIT_MULTIPLIER);
    scaled_limit
        .clamp(
            SHORT_QUERY_CONTENT_SCAN_MIN_ROWS,
            SHORT_QUERY_CONTENT_SCAN_MAX_ROWS,
        )
        .max(limit)
}

fn collect_search_results<P>(
    statement: &mut rusqlite::Statement<'_>,
    params: P,
) -> Result<Vec<DocumentSearchResult>, AppError>
where
    P: rusqlite::Params,
{
    let rows = statement
        .query_map(params, |row| {
            Ok((
                NodeRecord {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    title: row.get(2)?,
                    sort_order: row.get(3)?,
                    content: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    deleted_at: row.get(7)?,
                },
                row.get::<_, Option<String>>(8)?,
                row.get::<_, bool>(9)?,
            ))
        })
        .map_err(|source| AppError::sqlite("query SQLite document search results", source))?;

    let mut results = Vec::new();
    for row in rows {
        let (record, parent_title, content_matched) =
            row.map_err(|source| AppError::sqlite("read SQLite search result row", source))?;
        results.push(DocumentSearchResult {
            node: node_from_record(record)?,
            parent_title,
            content_matched,
        });
    }

    Ok(results)
}

fn like_contains_pattern(query: &str) -> Option<String> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }

    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for character in query.chars() {
        if matches!(character, '%' | '_' | '^') {
            pattern.push('^');
        }
        pattern.push(character);
    }
    pattern.push('%');
    Some(pattern)
}

fn is_short_trigram_query(query: &str) -> bool {
    query.trim().chars().count() < MIN_FTS_TRIGRAM_CHARS
}

fn fts_trigram_content_query(query: &str) -> Option<String> {
    fts_trigram_column_query("content", query)
}

fn fts_trigram_title_query(query: &str) -> Option<String> {
    fts_trigram_column_query("title", query)
}

fn fts_trigram_column_query(column: &str, query: &str) -> Option<String> {
    let query = query.trim();
    if query.chars().count() < MIN_FTS_TRIGRAM_CHARS
        || query.chars().any(|character| character == '\0')
    {
        return None;
    }

    let mut phrase = String::with_capacity(query.len() + 2);
    phrase.push('"');
    for character in query.chars() {
        if character == '"' {
            phrase.push('"');
        }
        phrase.push(character);
    }
    phrase.push('"');

    let mut fts_query = String::with_capacity(column.len() + " : ".len() + phrase.len());
    fts_query.push_str(column);
    fts_query.push_str(" : ");
    fts_query.push_str(&phrase);
    Some(fts_query)
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn fts_trigram_content_query_escapes_phrase_quotes() {
        assert_eq!(
            fts_trigram_content_query(r#"quoted "needle""#),
            Some(r#"content : "quoted ""needle""""#.to_string())
        );
    }

    #[test]
    fn fts_trigram_content_query_rejects_short_or_nul_queries() {
        assert_eq!(fts_trigram_content_query("ab"), None);
        assert_eq!(fts_trigram_content_query("abc\0def"), None);
    }

    #[test]
    fn fts_queries_match_titles_and_content_with_existing_result_shape() -> TestResult {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "
            CREATE TABLE nodes (
                id INTEGER PRIMARY KEY,
                parent_id INTEGER,
                title TEXT NOT NULL,
                sort_order INTEGER NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                deleted_at TEXT
            );
            CREATE VIRTUAL TABLE node_search_fts USING fts5(
                title,
                content,
                tokenize = 'trigram'
            );
            INSERT INTO nodes (
                id,
                parent_id,
                title,
                sort_order,
                content,
                created_at,
                updated_at,
                deleted_at
            ) VALUES
                (10, NULL, 'Parent', 0, 'parent content',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL),
                (1, 10, 'Alpha Child', 1, 'body does not match',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL),
                (2, NULL, 'Body Match', 2, 'alpha appears only in content',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL),
                (3, NULL, 'Alpha Deleted', 3, 'alpha deleted content',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z',
                    '2026-05-22T00:00:00Z'),
                (4, NULL, 'Zeta Alpha', 4, 'body does not match',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL),
                (5, NULL, 'Beta Alpha', 5, 'alpha appears in title and content',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL);
            INSERT INTO node_search_fts(rowid, title, content)
            SELECT id, title, content
            FROM nodes;
            ",
        )?;

        let limited_title_results = query_documents(&connection, "alpha", 2)?;

        assert_eq!(limited_title_results.len(), 2);
        assert_eq!(limited_title_results[0].node.title, "Alpha Child");
        assert!(!limited_title_results[0].content_matched);
        assert_eq!(limited_title_results[1].node.title, "Beta Alpha");
        assert!(limited_title_results[1].content_matched);

        let results = query_documents(&connection, "alpha", 4)?;

        assert_eq!(results.len(), 4);
        assert_eq!(results[0].node.title, "Alpha Child");
        assert_eq!(results[0].parent_title.as_deref(), Some("Parent"));
        assert!(!results[0].content_matched);
        assert_eq!(results[1].node.title, "Beta Alpha");
        assert!(results[1].content_matched);
        assert_eq!(results[2].node.title, "Zeta Alpha");
        assert!(!results[2].content_matched);
        assert_eq!(results[3].node.title, "Body Match");
        assert!(results[3].content_matched);
        Ok(())
    }

    #[test]
    fn fts_title_candidates_still_apply_like_literal_wildcards() -> TestResult {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "
            CREATE TABLE nodes (
                id INTEGER PRIMARY KEY,
                parent_id INTEGER,
                title TEXT NOT NULL,
                sort_order INTEGER NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                deleted_at TEXT
            );
            CREATE VIRTUAL TABLE node_search_fts USING fts5(
                title,
                content,
                tokenize = 'trigram'
            );
            INSERT INTO nodes (
                id,
                parent_id,
                title,
                sort_order,
                content,
                created_at,
                updated_at,
                deleted_at
            ) VALUES
                (1, NULL, 'save 50% now', 0, 'body does not match',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL),
                (2, NULL, 'save 500 now', 1, 'body does not match',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL),
                (3, NULL, 'save 50 percent now', 2, 'body does not match',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL);
            INSERT INTO node_search_fts(rowid, title, content)
            SELECT id, title, content
            FROM nodes;
            ",
        )?;

        let results = query_documents(&connection, "50%", 10)?;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].node.title, "save 50% now");
        assert!(!results[0].content_matched);
        Ok(())
    }

    #[test]
    fn short_queries_search_titles_then_content_like_fallback() -> TestResult {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "
            CREATE TABLE nodes (
                id INTEGER PRIMARY KEY,
                parent_id INTEGER,
                title TEXT NOT NULL,
                sort_order INTEGER NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                deleted_at TEXT
            );
            INSERT INTO nodes (
                id,
                parent_id,
                title,
                sort_order,
                content,
                created_at,
                updated_at,
                deleted_at
            ) VALUES
                (1, NULL, 'Body Only', 0, 'xy appears only in content',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL),
                (2, NULL, 'xy title', 1, 'body does not matter',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL),
                (3, NULL, 'xy both', 2, 'xy also appears in content',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL),
                (4, NULL, 'xy overflow', 3, 'xy also appears in content',
                    '2026-05-21T00:00:00Z', '2026-05-21T00:00:00Z', NULL);
            ",
        )?;

        let title_priority_results = query_documents(&connection, "xy", 2)?;

        assert_eq!(title_priority_results.len(), 2);
        assert_eq!(title_priority_results[0].node.title, "xy both");
        assert!(!title_priority_results[0].content_matched);
        assert_eq!(title_priority_results[1].node.title, "xy overflow");
        assert!(!title_priority_results[1].content_matched);

        let title_limit_results = query_documents(&connection, "xy", 3)?;

        assert_eq!(title_limit_results.len(), 3);
        assert_eq!(title_limit_results[0].node.title, "xy both");
        assert!(!title_limit_results[0].content_matched);
        assert_eq!(title_limit_results[1].node.title, "xy overflow");
        assert!(!title_limit_results[1].content_matched);
        assert_eq!(title_limit_results[2].node.title, "xy title");
        assert!(!title_limit_results[2].content_matched);

        let title_and_content_results = query_documents(&connection, "xy", 10)?;

        assert_eq!(title_and_content_results.len(), 4);
        assert_eq!(title_and_content_results[0].node.title, "xy both");
        assert!(!title_and_content_results[0].content_matched);
        assert_eq!(title_and_content_results[1].node.title, "xy overflow");
        assert!(!title_and_content_results[1].content_matched);
        assert_eq!(title_and_content_results[2].node.title, "xy title");
        assert!(!title_and_content_results[2].content_matched);
        assert_eq!(title_and_content_results[3].node.title, "Body Only");
        assert!(title_and_content_results[3].content_matched);
        Ok(())
    }

    #[test]
    fn short_queries_bound_content_like_fallback_candidates() -> TestResult {
        let connection = Connection::open_in_memory()?;
        connection.execute_batch(
            "
            CREATE TABLE nodes (
                id INTEGER PRIMARY KEY,
                parent_id INTEGER,
                title TEXT NOT NULL,
                sort_order INTEGER NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                deleted_at TEXT
            );
            ",
        )?;

        let timestamp = "2026-05-21T00:00:00Z";
        let content_scan_limit = short_query_content_scan_limit(1);
        for offset in 0..content_scan_limit {
            let id = offset + 1;
            let title = format!("candidate {offset:04}");
            connection.execute(
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
                ) VALUES (?1, NULL, ?2, ?3, 'body does not match', ?4, ?4, NULL)
                ",
                rusqlite::params![id, title, id, timestamp],
            )?;
        }
        connection.execute(
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
            ) VALUES (?1, NULL, 'zz target', ?1, 'xy appears after bounded candidates', ?2, ?2, NULL)
            ",
            rusqlite::params![content_scan_limit + 1, timestamp],
        )?;

        let results = query_documents(&connection, "xy", 1)?;

        assert!(results.is_empty());
        Ok(())
    }
}
