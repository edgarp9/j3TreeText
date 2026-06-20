use std::collections::{HashMap, HashSet};

use rusqlite::{
    params, params_from_iter, OptionalExtension, Params, Transaction, TransactionBehavior,
};

use crate::domain::{
    Document, DomainError, Node, NodeSiblingOrderUpdate, SiblingMoveDirection, ROOT_NODE_ID,
};
use crate::error::{AppError, SqliteUserMessage};

use super::node::{
    current_timestamp, insert_node, next_timestamp_after, node_from_record, NodeRecord,
};
use super::{tree, DeleteNodeUpdate, RenamedNodeUpdate, SqliteDocumentRepository};

const NODE_CONTENT_BATCH_SIZE: usize = 900;

impl SqliteDocumentRepository {
    pub fn create_child_node(
        &mut self,
        parent_id: i64,
        base_title: &str,
    ) -> Result<Node, AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| AppError::sqlite("start create node transaction", source))?;

        tree::ensure_active_node(&transaction, parent_id)?;

        let base_title = tree::normalize_title_input(base_title)?;
        let title = tree::unique_child_title(&transaction, Some(parent_id), &base_title)?;
        let id = tree::next_node_id(&transaction)?;
        let sort_order = tree::next_child_sort_order(&transaction, Some(parent_id))?;
        let now = current_timestamp(&transaction)?;
        let node = Node {
            id,
            parent_id: Some(parent_id),
            title,
            sort_order,
            content: String::new(),
            created_at: now.clone(),
            updated_at: now,
            deleted_at: None,
        };
        node.validate()?;
        insert_node(&transaction, &node)?;

        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit create node transaction", source))?;
        Ok(node)
    }

    pub fn rename_node(&mut self, node_id: i64, title: &str) -> Result<(), AppError> {
        self.rename_node_update(node_id, title).map(|_| ())
    }

    pub fn rename_node_update(
        &mut self,
        node_id: i64,
        title: &str,
    ) -> Result<RenamedNodeUpdate, AppError> {
        let title = tree::normalize_title_input(title)?;
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| AppError::sqlite("start rename node transaction", source))?;

        let parent_id = tree::active_node_parent_id(&transaction, node_id)?;
        tree::ensure_unique_child_title(&transaction, parent_id, &title, Some(node_id))?;
        let now = metadata_timestamp_after_active_node(&transaction, node_id)?;

        let updated = transaction
            .execute(
                "
                UPDATE nodes
                SET title = ?1, updated_at = ?2
                WHERE id = ?3 AND deleted_at IS NULL
                ",
                params![&title, &now, node_id],
            )
            .map_err(|source| AppError::sqlite("rename SQLite node", source))?;

        if updated == 0 {
            return Err(DomainError::NodeNotFound { node_id }.into());
        }

        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit rename node transaction", source))?;

        Ok(RenamedNodeUpdate {
            title,
            updated_at: now,
        })
    }

    pub fn update_document_content(
        &mut self,
        node_id: i64,
        content: &str,
        expected_updated_at: &str,
    ) -> Result<String, AppError> {
        if content.contains('\0') {
            return Err(DomainError::EmbeddedNulContent { node_id }.into());
        }

        let transaction = self.connection.transaction().map_err(|source| {
            AppError::sqlite_with_user_message(
                "start save document content transaction",
                SqliteUserMessage::SaveDocumentContent,
                source,
            )
        })?;

        tree::ensure_active_node(&transaction, node_id)?;

        let now = next_timestamp_after(&transaction, expected_updated_at)?;
        mark_node_search_content_validated(&transaction, node_id, &now)?;
        let updated = transaction
            .execute(
                "
                UPDATE nodes
                SET content = ?1, updated_at = ?2
                WHERE id = ?3
                    AND deleted_at IS NULL
                    AND updated_at = ?4
                ",
                params![content, &now, node_id, expected_updated_at],
            )
            .map_err(|source| {
                AppError::sqlite_with_user_message(
                    "save document content",
                    SqliteUserMessage::SaveDocumentContent,
                    source,
                )
            })?;

        if updated == 0 {
            return Err(DomainError::DocumentSaveConflict { node_id }.into());
        }

        transaction.commit().map_err(|source| {
            AppError::sqlite_with_user_message(
                "commit save document content transaction",
                SqliteUserMessage::SaveDocumentContent,
                source,
            )
        })?;
        Ok(now)
    }

    pub fn append_document_content(
        &mut self,
        node_id: i64,
        content: &str,
        expected_updated_at: &str,
    ) -> Result<String, AppError> {
        self.append_document_content_with_existing_content_check(
            node_id,
            content,
            expected_updated_at,
            ExistingContentNulCheck::ScanAtExpectedTimestamp,
        )
    }

    /// Appends content after `load_active_node_content_byte_len` has validated the existing content.
    pub fn append_document_content_with_known_valid_existing_content(
        &mut self,
        node_id: i64,
        content: &str,
        expected_updated_at: &str,
    ) -> Result<String, AppError> {
        self.append_document_content_with_existing_content_check(
            node_id,
            content,
            expected_updated_at,
            ExistingContentNulCheck::AlreadyValidated,
        )
    }

    fn append_document_content_with_existing_content_check(
        &mut self,
        node_id: i64,
        content: &str,
        expected_updated_at: &str,
        existing_content_nul_check: ExistingContentNulCheck,
    ) -> Result<String, AppError> {
        if content.contains('\0') {
            return Err(DomainError::EmbeddedNulContent { node_id }.into());
        }

        let transaction = self.connection.transaction().map_err(|source| {
            AppError::sqlite_with_user_message(
                "start save document content transaction",
                SqliteUserMessage::SaveDocumentContent,
                source,
            )
        })?;

        tree::ensure_active_node(&transaction, node_id)?;

        let now = next_timestamp_after(&transaction, expected_updated_at)?;
        mark_node_search_content_validated(&transaction, node_id, &now)?;
        let updated = match existing_content_nul_check {
            ExistingContentNulCheck::AlreadyValidated => transaction.execute(
                "
                UPDATE nodes
                SET content = content || ?1, updated_at = ?2
                WHERE id = ?3
                    AND deleted_at IS NULL
                    AND updated_at = ?4
                ",
                params![content, &now, node_id, expected_updated_at],
            ),
            ExistingContentNulCheck::ScanAtExpectedTimestamp => transaction.execute(
                "
                UPDATE nodes
                SET content = content || ?1, updated_at = ?2
                WHERE id = ?3
                    AND deleted_at IS NULL
                    AND updated_at = ?4
                    AND instr(content, char(0)) = 0
                ",
                params![content, &now, node_id, expected_updated_at],
            ),
        }
        .map_err(|source| {
            AppError::sqlite_with_user_message(
                "save document content",
                SqliteUserMessage::SaveDocumentContent,
                source,
            )
        })?;

        if updated == 0 {
            if existing_content_nul_check == ExistingContentNulCheck::ScanAtExpectedTimestamp
                && active_content_has_embedded_nul_at_timestamp(
                    &transaction,
                    node_id,
                    expected_updated_at,
                )?
            {
                return Err(DomainError::EmbeddedNulContent { node_id }.into());
            }
            return Err(DomainError::DocumentSaveConflict { node_id }.into());
        }

        transaction.commit().map_err(|source| {
            AppError::sqlite_with_user_message(
                "commit save document content transaction",
                SqliteUserMessage::SaveDocumentContent,
                source,
            )
        })?;
        Ok(now)
    }

    pub fn create_document_with_content(
        &mut self,
        parent_id: i64,
        base_title: &str,
        content: &str,
    ) -> Result<Node, AppError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| AppError::sqlite("start create document copy transaction", source))?;

        tree::ensure_active_node(&transaction, parent_id)?;

        let base_title = tree::normalize_title_input(base_title)?;
        let title = tree::unique_child_title(&transaction, Some(parent_id), &base_title)?;
        let id = tree::next_node_id(&transaction)?;
        let sort_order = tree::next_child_sort_order(&transaction, Some(parent_id))?;
        let now = current_timestamp(&transaction)?;
        let node = Node {
            id,
            parent_id: Some(parent_id),
            title,
            sort_order,
            content: content.to_owned(),
            created_at: now.clone(),
            updated_at: now,
            deleted_at: None,
        };
        node.validate()?;
        mark_node_search_content_validated(&transaction, id, &node.updated_at)?;
        insert_node(&transaction, &node)?;

        transaction.commit().map_err(|source| {
            AppError::sqlite("commit create document copy transaction", source)
        })?;
        Ok(node)
    }

    pub fn move_node_to_parent_end(
        &mut self,
        node_id: i64,
        parent_id: i64,
    ) -> Result<(), AppError> {
        self.move_node_to_parent_end_update(node_id, parent_id)
            .map(|_| ())
    }

    pub fn move_node_to_parent_end_update(
        &mut self,
        node_id: i64,
        parent_id: i64,
    ) -> Result<Vec<NodeSiblingOrderUpdate>, AppError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| AppError::sqlite("start move node transaction", source))?;

        let source = tree::active_node_summary(&transaction, node_id)?;
        tree::ensure_movable_node(node_id, source.parent_id)?;
        tree::ensure_active_node(&transaction, parent_id)?;
        tree::ensure_node_can_move_to_parent(&transaction, node_id, parent_id)?;
        tree::ensure_unique_child_title(
            &transaction,
            Some(parent_id),
            &source.title,
            Some(node_id),
        )?;

        let old_parent_id = source.parent_id;
        let new_parent_id = Some(parent_id);
        let now = metadata_timestamp_after_active_sibling_groups(
            &transaction,
            node_id,
            old_parent_id,
            new_parent_id,
        )?;
        let sibling_orders = if old_parent_id == new_parent_id {
            tree::move_node_to_current_parent_end(
                &transaction,
                node_id,
                new_parent_id,
                &source.title,
                source.sort_order,
                &now,
            )?
        } else {
            let new_sort_order = tree::next_child_sort_order(&transaction, new_parent_id)?;
            tree::update_node_parent(&transaction, node_id, new_parent_id, new_sort_order, &now)?;

            let mut sibling_orders = tree::shift_sibling_sort_orders_after_removal(
                &transaction,
                old_parent_id,
                source.sort_order,
                None,
                &now,
            )?;
            sibling_orders.push(NodeSiblingOrderUpdate {
                node_id,
                parent_id: new_parent_id,
                sort_order: new_sort_order,
                updated_at: now.clone(),
            });
            sibling_orders
        };

        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit move node transaction", source))?;
        Ok(sibling_orders)
    }

    pub fn move_node_within_parent(
        &mut self,
        node_id: i64,
        direction: SiblingMoveDirection,
    ) -> Result<(), AppError> {
        self.move_node_within_parent_update(node_id, direction)
            .map(|_| ())
    }

    pub fn move_node_within_parent_update(
        &mut self,
        node_id: i64,
        direction: SiblingMoveDirection,
    ) -> Result<Vec<NodeSiblingOrderUpdate>, AppError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| AppError::sqlite("start reorder node transaction", source))?;

        let source = tree::active_node_summary(&transaction, node_id)?;
        tree::ensure_movable_node(node_id, source.parent_id)?;
        let Some(neighbor) = tree::adjacent_active_sibling(
            &transaction,
            source.parent_id,
            node_id,
            &source.title,
            source.sort_order,
            &direction,
        )?
        else {
            return Ok(Vec::new());
        };

        if neighbor.sort_order == source.sort_order {
            let now = metadata_timestamp_after_active_siblings(&transaction, source.parent_id)?;
            let sibling_orders = tree::reorder_duplicate_sibling_sort_order(
                &transaction,
                source.parent_id,
                node_id,
                neighbor.node_id,
                source.sort_order,
                &direction,
                &now,
            )?;

            transaction
                .commit()
                .map_err(|source| AppError::sqlite("commit reorder node transaction", source))?;
            return Ok(sibling_orders);
        }

        let now = metadata_timestamp_after_active_siblings(&transaction, source.parent_id)?;
        tree::update_sibling_sort_order(
            &transaction,
            node_id,
            source.parent_id,
            neighbor.sort_order,
            &now,
        )?;
        tree::update_sibling_sort_order(
            &transaction,
            neighbor.node_id,
            source.parent_id,
            source.sort_order,
            &now,
        )?;

        let sibling_orders = match direction {
            SiblingMoveDirection::Up => vec![
                NodeSiblingOrderUpdate {
                    node_id,
                    parent_id: source.parent_id,
                    sort_order: neighbor.sort_order,
                    updated_at: now.clone(),
                },
                NodeSiblingOrderUpdate {
                    node_id: neighbor.node_id,
                    parent_id: source.parent_id,
                    sort_order: source.sort_order,
                    updated_at: now.clone(),
                },
            ],
            SiblingMoveDirection::Down => vec![
                NodeSiblingOrderUpdate {
                    node_id: neighbor.node_id,
                    parent_id: source.parent_id,
                    sort_order: source.sort_order,
                    updated_at: now.clone(),
                },
                NodeSiblingOrderUpdate {
                    node_id,
                    parent_id: source.parent_id,
                    sort_order: neighbor.sort_order,
                    updated_at: now.clone(),
                },
            ],
        };

        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit reorder node transaction", source))?;
        Ok(sibling_orders)
    }

    pub fn soft_delete_node_cascade(&mut self, node_id: i64) -> Result<(), AppError> {
        self.soft_delete_node_cascade_update(node_id).map(|_| ())
    }

    pub fn stage_active_subtree_node_ids_for_delete(
        &mut self,
        node_id: i64,
    ) -> Result<Vec<i64>, AppError> {
        if node_id == ROOT_NODE_ID {
            return Err(DomainError::CannotDeleteRoot.into());
        }

        let transaction = self.connection.transaction().map_err(|source| {
            AppError::sqlite("start active subtree staging transaction", source)
        })?;
        let subtree_len = tree::stage_active_subtree(&transaction, node_id)?;
        if subtree_len == 0 {
            return Err(DomainError::NodeNotFound { node_id }.into());
        }

        let node_ids = tree::staged_subtree_node_ids(&transaction)?;
        transaction.commit().map_err(|source| {
            AppError::sqlite("commit active subtree staging transaction", source)
        })?;
        Ok(node_ids)
    }

    pub fn soft_delete_node_cascade_update(
        &mut self,
        node_id: i64,
    ) -> Result<DeleteNodeUpdate, AppError> {
        self.soft_delete_node_cascade_update_with_staged(node_id, None)
    }

    pub fn soft_delete_node_cascade_update_from_staged_active_subtree(
        &mut self,
        node_id: i64,
        staged_node_ids: &[i64],
    ) -> Result<DeleteNodeUpdate, AppError> {
        self.soft_delete_node_cascade_update_with_staged(node_id, Some(staged_node_ids))
    }

    fn soft_delete_node_cascade_update_with_staged(
        &mut self,
        node_id: i64,
        staged_node_ids: Option<&[i64]>,
    ) -> Result<DeleteNodeUpdate, AppError> {
        if node_id == ROOT_NODE_ID {
            return Err(DomainError::CannotDeleteRoot.into());
        }

        let transaction = self
            .connection
            .transaction()
            .map_err(|source| AppError::sqlite("start delete node transaction", source))?;
        let (parent_id, sort_order) = tree::active_node_parent_sort_order(&transaction, node_id)?;
        if parent_id.is_none() {
            return Err(DomainError::CannotDeleteRoot.into());
        }

        let use_staged_subtree = match staged_node_ids {
            Some(node_ids) => tree::staged_active_subtree_matches(&transaction, node_id, node_ids)?,
            None => false,
        };
        let (now, removed_node_ids) = if use_staged_subtree {
            tree::soft_delete_staged_active_subtree_node_ids(&transaction, node_id, parent_id)?
        } else {
            tree::soft_delete_active_subtree_node_ids(&transaction, node_id, parent_id)?
        };

        if removed_node_ids.is_empty() {
            return Err(DomainError::NodeNotFound { node_id }.into());
        }

        let sibling_orders = tree::shift_sibling_sort_orders_after_removal(
            &transaction,
            parent_id,
            sort_order,
            None,
            &now,
        )?;

        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit delete node transaction", source))?;

        Ok(DeleteNodeUpdate {
            removed_node_ids,
            sibling_orders,
        })
    }

    pub fn load_active_subtree_node_ids(&self, node_id: i64) -> Result<Vec<i64>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "
                WITH RECURSIVE subtree(id) AS (
                    SELECT id
                    FROM nodes
                    WHERE id = ?1 AND deleted_at IS NULL

                    UNION ALL

                    SELECT child.id
                    FROM nodes child
                    INNER JOIN subtree ON child.parent_id = subtree.id
                    WHERE child.deleted_at IS NULL
                )
                SELECT id
                FROM subtree
                ",
            )
            .map_err(|source| AppError::sqlite("prepare active subtree node id query", source))?;

        let rows = statement
            .query_map(params![node_id], |row| row.get(0))
            .map_err(|source| AppError::sqlite("query active SQLite subtree node ids", source))?;

        let node_ids = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| AppError::sqlite("read active SQLite subtree node ids", source))?;
        if node_ids.is_empty() {
            return Err(DomainError::NodeNotFound { node_id }.into());
        }
        Ok(node_ids)
    }

    pub fn load_deleted_nodes(&self) -> Result<Vec<Node>, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT id, parent_id, title, sort_order, '' AS content, created_at, updated_at, deleted_at
                FROM nodes
                WHERE deleted_at IS NOT NULL
                ORDER BY deleted_at DESC, parent_id IS NOT NULL, parent_id, sort_order, title, id
                ",
            )
            .map_err(|source| AppError::sqlite("prepare deleted node query", source))?;

        let rows = statement
            .query_map([], |row| {
                Ok(NodeRecord {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    title: row.get(2)?,
                    sort_order: row.get(3)?,
                    content: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    deleted_at: row.get(7)?,
                })
            })
            .map_err(|source| AppError::sqlite("query deleted SQLite nodes", source))?;

        let mut nodes = Vec::new();
        for row in rows {
            let record =
                row.map_err(|source| AppError::sqlite("read deleted SQLite node row", source))?;
            nodes.push(node_from_record(record)?);
        }

        Ok(nodes)
    }

    pub fn load_deleted_node(&self, node_id: i64) -> Result<Node, AppError> {
        let Some(record) = self
            .connection
            .query_row(
                "
                SELECT id, parent_id, title, sort_order, content, created_at, updated_at, deleted_at
                FROM nodes
                WHERE id = ?1 AND deleted_at IS NOT NULL
                ",
                params![node_id],
                |row| {
                    Ok(NodeRecord {
                        id: row.get(0)?,
                        parent_id: row.get(1)?,
                        title: row.get(2)?,
                        sort_order: row.get(3)?,
                        content: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                        deleted_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|source| AppError::sqlite("read deleted SQLite node", source))?
        else {
            return Err(DomainError::NodeNotFound { node_id }.into());
        };

        node_from_record(record)
    }

    pub fn restore_deleted_node_cascade(&mut self, node_id: i64) -> Result<(), AppError> {
        self.restore_deleted_node_cascade_update(node_id)
            .map(|_| ())
    }

    pub fn restore_deleted_node_cascade_update(
        &mut self,
        node_id: i64,
    ) -> Result<Vec<Node>, AppError> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|source| AppError::sqlite("start restore node transaction", source))?;

        let deleted_node = tree::deleted_node_summary(&transaction, node_id)?;
        let target_parent_id =
            tree::restore_target_parent_id(&transaction, deleted_node.parent_id)?;
        let title =
            tree::unique_restored_child_title(&transaction, target_parent_id, &deleted_node.title)?;
        let sort_order = tree::next_child_sort_order(&transaction, target_parent_id)?;
        let restored_nodes = tree::restore_deleted_subtree_metadata_nodes(
            &transaction,
            node_id,
            target_parent_id,
            &title,
            sort_order,
        )?;

        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit restore node transaction", source))?;

        Ok(restored_nodes)
    }

    pub fn permanently_delete_node_cascade(&mut self, node_id: i64) -> Result<(), AppError> {
        let transaction = self.connection.transaction().map_err(|source| {
            AppError::sqlite("start permanent delete node transaction", source)
        })?;

        tree::ensure_deleted_node(&transaction, node_id)?;
        let deleted = transaction
            .execute(
                "
                WITH RECURSIVE subtree(id) AS (
                    SELECT id
                    FROM nodes
                    WHERE id = ?1 AND deleted_at IS NOT NULL

                    UNION ALL

                    SELECT child.id
                    FROM nodes child
                    INNER JOIN subtree ON child.parent_id = subtree.id
                    WHERE child.deleted_at IS NOT NULL
                )
                DELETE FROM nodes
                WHERE id IN (SELECT id FROM subtree)
                    AND deleted_at IS NOT NULL
                ",
                params![node_id],
            )
            .map_err(|source| AppError::sqlite("permanently delete SQLite subtree", source))?;
        if deleted == 0 {
            return Err(DomainError::NodeNotDeleted { node_id }.into());
        }

        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit permanent delete node transaction", source))
    }

    pub fn load_document(&self) -> Result<Document, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT id, parent_id, title, sort_order, content, created_at, updated_at, deleted_at
                FROM nodes
                WHERE deleted_at IS NULL
                ORDER BY parent_id IS NOT NULL, parent_id, sort_order, title, id
                ",
            )
            .map_err(|source| AppError::sqlite("prepare node query", source))?;

        let rows = statement
            .query_map([], |row| {
                Ok(NodeRecord {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    title: row.get(2)?,
                    sort_order: row.get(3)?,
                    content: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    deleted_at: row.get(7)?,
                })
            })
            .map_err(|source| AppError::sqlite("query SQLite nodes", source))?;

        let mut nodes = Vec::new();
        for row in rows {
            let record = row.map_err(|source| AppError::sqlite("read SQLite node row", source))?;
            nodes.push(node_from_record(record)?);
        }

        Document::new(nodes).map_err(AppError::from)
    }

    /// Loads active node metadata in UI display order.
    ///
    /// The Win32 active tree builder preserves this order to avoid sorting the full tree again.
    pub fn load_document_metadata(&self) -> Result<Document, AppError> {
        let mut statement = self
            .connection
            .prepare(
                "
                SELECT id, parent_id, title, sort_order, '' AS content, created_at, updated_at, deleted_at
                FROM nodes
                WHERE deleted_at IS NULL
                ORDER BY parent_id IS NOT NULL, parent_id, sort_order, title, id
                ",
            )
            .map_err(|source| AppError::sqlite("prepare node metadata query", source))?;

        let rows = statement
            .query_map([], |row| {
                Ok(NodeRecord {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    title: row.get(2)?,
                    sort_order: row.get(3)?,
                    content: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    deleted_at: row.get(7)?,
                })
            })
            .map_err(|source| AppError::sqlite("query SQLite node metadata", source))?;

        let mut nodes = Vec::new();
        for row in rows {
            let record =
                row.map_err(|source| AppError::sqlite("read SQLite node metadata row", source))?;
            nodes.push(node_from_record(record)?);
        }

        Document::new(nodes).map_err(AppError::from)
    }

    pub fn load_active_node(&self, node_id: i64) -> Result<Option<Node>, AppError> {
        let record = self
            .connection
            .query_row(
                "
                SELECT id, parent_id, title, sort_order, content, created_at, updated_at, deleted_at
                FROM nodes
                WHERE id = ?1 AND deleted_at IS NULL
                ",
                params![node_id],
                |row| {
                    Ok(NodeRecord {
                        id: row.get(0)?,
                        parent_id: row.get(1)?,
                        title: row.get(2)?,
                        sort_order: row.get(3)?,
                        content: row.get(4)?,
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                        deleted_at: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(|source| AppError::sqlite("read active SQLite node", source))?;

        record.map(node_from_record).transpose()
    }

    pub fn load_active_node_updated_at(&self, node_id: i64) -> Result<String, AppError> {
        self.connection
            .query_row(
                "
                SELECT updated_at
                FROM nodes
                WHERE id = ?1 AND deleted_at IS NULL
                ",
                params![node_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|source| AppError::sqlite("read active SQLite node updated_at", source))?
            .ok_or_else(|| DomainError::NodeNotFound { node_id }.into())
    }

    pub fn load_active_node_content_byte_len(
        &self,
        node_id: i64,
    ) -> Result<(u64, String), AppError> {
        let Some((content_byte_len, updated_at, has_embedded_nul)) = self
            .connection
            .query_row(
                "
                SELECT
                    length(CAST(content AS BLOB)),
                    updated_at,
                    instr(CAST(content AS BLOB), X'00') > 0
                FROM nodes
                WHERE id = ?1 AND deleted_at IS NULL
                ",
                params![node_id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? != 0,
                    ))
                },
            )
            .optional()
            .map_err(|source| AppError::sqlite("read active SQLite node content length", source))?
        else {
            return Err(DomainError::NodeNotFound { node_id }.into());
        };

        if has_embedded_nul {
            return Err(DomainError::EmbeddedNulContent { node_id }.into());
        }

        Ok((content_byte_len as u64, updated_at))
    }

    pub fn load_active_node_content_if_present(
        &self,
        node_id: i64,
    ) -> Result<Option<(String, String)>, AppError> {
        let Some((content, updated_at)) = self
            .connection
            .query_row(
                "
                SELECT content, updated_at
                FROM nodes
                WHERE id = ?1 AND deleted_at IS NULL
                ",
                params![node_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|source| AppError::sqlite("read SQLite node contents", source))?
        else {
            return Ok(None);
        };

        if content.contains('\0') {
            return Err(DomainError::EmbeddedNulContent { node_id }.into());
        }

        Ok(Some((content, updated_at)))
    }

    pub fn load_active_node_contents_if_present(
        &self,
        node_ids: &[i64],
    ) -> Result<HashMap<i64, (String, String)>, AppError> {
        self.load_node_contents_by_deleted_state(
            node_ids,
            "deleted_at IS NULL",
            "read SQLite node contents",
        )
    }

    pub fn load_active_node_content(&self, node_id: i64) -> Result<(String, String), AppError> {
        self.load_active_node_content_if_present(node_id)?
            .ok_or_else(|| DomainError::NodeNotFound { node_id }.into())
    }

    pub fn load_deleted_node_content(&self, node_id: i64) -> Result<(String, String), AppError> {
        let mut contents = self.load_deleted_node_contents(&[node_id])?;
        contents
            .remove(&node_id)
            .ok_or_else(|| DomainError::NodeNotFound { node_id }.into())
    }

    pub fn load_deleted_node_contents(
        &self,
        node_ids: &[i64],
    ) -> Result<HashMap<i64, (String, String)>, AppError> {
        self.load_node_contents_by_deleted_state(
            node_ids,
            "deleted_at IS NOT NULL",
            "read deleted SQLite node contents",
        )
    }

    fn load_node_contents_by_deleted_state(
        &self,
        node_ids: &[i64],
        deleted_state_predicate: &str,
        context: &'static str,
    ) -> Result<HashMap<i64, (String, String)>, AppError> {
        let mut contents = HashMap::with_capacity(node_ids.len());
        let mut unique_node_ids = Vec::with_capacity(node_ids.len());
        let mut seen_node_ids = HashSet::with_capacity(node_ids.len());
        for &node_id in node_ids {
            if seen_node_ids.insert(node_id) {
                unique_node_ids.push(node_id);
            }
        }

        for chunk in unique_node_ids.chunks(NODE_CONTENT_BATCH_SIZE) {
            if chunk.is_empty() {
                continue;
            }

            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "
                SELECT id, content, updated_at
                FROM nodes
                WHERE id IN ({placeholders}) AND {deleted_state_predicate}
                "
            );
            let mut statement = self
                .connection
                .prepare(&sql)
                .map_err(|source| AppError::sqlite(context, source))?;
            let rows = statement
                .query_map(params_from_iter(chunk.iter().copied()), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|source| AppError::sqlite(context, source))?;

            for row in rows {
                let (node_id, content, updated_at) =
                    row.map_err(|source| AppError::sqlite(context, source))?;
                if content.contains('\0') {
                    return Err(DomainError::EmbeddedNulContent { node_id }.into());
                }

                contents.insert(node_id, (content, updated_at));
            }
        }

        Ok(contents)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExistingContentNulCheck {
    ScanAtExpectedTimestamp,
    AlreadyValidated,
}

fn mark_node_search_content_validated(
    transaction: &Transaction<'_>,
    node_id: i64,
    updated_at: &str,
) -> Result<(), AppError> {
    transaction
        .execute(
            "
            INSERT OR REPLACE INTO node_search_validated_content (node_id, updated_at)
            VALUES (?1, ?2)
            ",
            params![node_id, updated_at],
        )
        .map_err(|source| AppError::sqlite("mark SQLite node content as validated", source))?;
    Ok(())
}

fn active_content_has_embedded_nul_at_timestamp(
    transaction: &Transaction<'_>,
    node_id: i64,
    updated_at: &str,
) -> Result<bool, AppError> {
    let has_embedded_nul = transaction
        .query_row(
            "
            SELECT instr(content, char(0)) > 0
            FROM nodes
            WHERE id = ?1
                AND deleted_at IS NULL
                AND updated_at = ?2
            ",
            params![node_id, updated_at],
            |row| row.get::<_, bool>(0),
        )
        .optional()
        .map_err(|source| AppError::sqlite("read SQLite node content NUL state", source))?;

    Ok(has_embedded_nul.unwrap_or(false))
}

fn metadata_timestamp_after_active_node(
    transaction: &Transaction<'_>,
    node_id: i64,
) -> Result<String, AppError> {
    metadata_timestamp_after_max_updated_at(
        transaction,
        "
        SELECT MAX(updated_at)
        FROM nodes
        WHERE id = ?1 AND deleted_at IS NULL
        ",
        params![node_id],
        "read active SQLite node timestamp for metadata update",
    )
}

fn metadata_timestamp_after_active_sibling_groups(
    transaction: &Transaction<'_>,
    node_id: i64,
    old_parent_id: Option<i64>,
    new_parent_id: Option<i64>,
) -> Result<String, AppError> {
    metadata_timestamp_after_max_updated_at(
        transaction,
        "
        SELECT MAX(updated_at)
        FROM nodes
        WHERE deleted_at IS NULL
            AND (
                id = ?1
                OR parent_id IS ?2
                OR parent_id IS ?3
            )
        ",
        params![node_id, old_parent_id, new_parent_id],
        "read active SQLite sibling timestamps for metadata update",
    )
}

fn metadata_timestamp_after_active_siblings(
    transaction: &Transaction<'_>,
    parent_id: Option<i64>,
) -> Result<String, AppError> {
    metadata_timestamp_after_max_updated_at(
        transaction,
        "
        SELECT MAX(updated_at)
        FROM nodes
        WHERE deleted_at IS NULL AND parent_id IS ?1
        ",
        params![parent_id],
        "read active SQLite sibling timestamps for metadata reorder",
    )
}

fn metadata_timestamp_after_max_updated_at<P>(
    transaction: &Transaction<'_>,
    sql: &str,
    params: P,
    context: &'static str,
) -> Result<String, AppError>
where
    P: Params,
{
    let previous_updated_at = transaction
        .query_row(sql, params, |row| row.get::<_, Option<String>>(0))
        .map_err(|source| AppError::sqlite(context, source))?;

    match previous_updated_at {
        Some(previous_updated_at) => next_timestamp_after(transaction, &previous_updated_at),
        None => current_timestamp(transaction),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn load_active_node_content_byte_len_returns_utf8_byte_count() -> Result<(), AppError> {
        let (repository, updated_at) = repository_with_content("한a")?;

        let (content_byte_len, loaded_updated_at) =
            repository.load_active_node_content_byte_len(1)?;

        assert_eq!(content_byte_len, "한a".len() as u64);
        assert_eq!(loaded_updated_at, updated_at);
        Ok(())
    }

    #[test]
    fn append_document_content_concatenates_existing_content() -> Result<(), AppError> {
        let (mut repository, expected_updated_at) = repository_with_content("hello")?;

        let updated_at = repository.append_document_content(1, " world", &expected_updated_at)?;
        let (content, loaded_updated_at) = repository.load_active_node_content(1)?;

        assert_eq!(content, "hello world");
        assert_eq!(loaded_updated_at, updated_at);
        assert!(updated_at.as_str() > expected_updated_at.as_str());
        Ok(())
    }

    #[test]
    fn append_document_content_uses_known_valid_existing_content() -> Result<(), AppError> {
        let (mut repository, _) = repository_with_content("hello")?;
        let (_, expected_updated_at) = repository.load_active_node_content_byte_len(1)?;

        let updated_at = repository.append_document_content_with_known_valid_existing_content(
            1,
            " world",
            &expected_updated_at,
        )?;
        let (content, loaded_updated_at) = repository.load_active_node_content(1)?;

        assert_eq!(content, "hello world");
        assert_eq!(loaded_updated_at, updated_at);
        assert!(updated_at.as_str() > expected_updated_at.as_str());
        Ok(())
    }

    #[test]
    fn append_document_content_rejects_stored_embedded_nul_content() -> Result<(), AppError> {
        let (mut repository, expected_updated_at) = repository_with_content("Bad\0Content")?;

        let error = match repository.append_document_content(1, " tail", &expected_updated_at) {
            Ok(_) => return Err(DomainError::EmbeddedNulContent { node_id: 1 }.into()),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AppError::Domain(DomainError::EmbeddedNulContent { node_id: 1 })
        ));
        Ok(())
    }

    #[test]
    fn move_node_within_parent_reorders_duplicate_sort_order_without_updating_prefix(
    ) -> Result<(), AppError> {
        let parent_id = Some(10);
        let (mut repository, created_at) = repository_with_sibling_orders(&[
            (1, parent_id, "Before", 0),
            (2, parent_id, "A", 5),
            (3, parent_id, "B", 5),
            (4, parent_id, "C", 5),
            (5, parent_id, "After", 6),
        ])?;

        let updates = repository.move_node_within_parent_update(3, SiblingMoveDirection::Down)?;

        assert_eq!(
            updates
                .iter()
                .map(|update| (update.node_id, update.parent_id, update.sort_order))
                .collect::<Vec<_>>(),
            vec![(4, parent_id, 6), (3, parent_id, 7), (5, parent_id, 8)]
        );
        let updated_at = updates
            .first()
            .map(|update| update.updated_at.as_str())
            .ok_or_else(|| {
                AppError::internal_consistency(
                    "test duplicate sibling reorder",
                    "expected sibling order updates",
                )
            })?;
        assert!(updated_at > created_at.as_str());
        assert!(updates.iter().all(|update| update.updated_at == updated_at));
        assert_eq!(
            active_node_sort_orders(&repository.connection, parent_id)?,
            vec![(1, 0), (2, 5), (4, 6), (3, 7), (5, 8)]
        );
        assert_eq!(
            active_node_updated_ats(&repository.connection, parent_id)?,
            vec![
                (1, created_at.clone()),
                (2, created_at),
                (3, updated_at.to_owned()),
                (4, updated_at.to_owned()),
                (5, updated_at.to_owned()),
            ]
        );
        Ok(())
    }

    #[test]
    fn metadata_timestamp_after_active_node_advances_existing_token() -> Result<(), AppError> {
        let mut connection = Connection::open_in_memory()
            .map_err(|source| AppError::sqlite("open test SQLite connection", source))?;
        let transaction = connection
            .transaction()
            .map_err(|source| AppError::sqlite("start test SQLite transaction", source))?;

        transaction
            .execute(
                "
                CREATE TABLE nodes (
                    id INTEGER PRIMARY KEY,
                    parent_id INTEGER,
                    updated_at TEXT NOT NULL,
                    deleted_at TEXT
                )
                ",
                [],
            )
            .map_err(|source| AppError::sqlite("create test SQLite nodes table", source))?;

        let previous_updated_at = current_timestamp(&transaction)?;
        transaction
            .execute(
                "
                INSERT INTO nodes (
                    id,
                    parent_id,
                    updated_at,
                    deleted_at
                )
                VALUES (?1, ?2, ?3, NULL)
                ",
                params![1_i64, ROOT_NODE_ID, &previous_updated_at],
            )
            .map_err(|source| AppError::sqlite("insert test SQLite node", source))?;

        let next_updated_at = metadata_timestamp_after_active_node(&transaction, 1)?;

        assert!(next_updated_at.as_str() > previous_updated_at.as_str());
        Ok(())
    }

    fn repository_with_content(
        content: &str,
    ) -> Result<(SqliteDocumentRepository, String), AppError> {
        let mut connection = Connection::open_in_memory()
            .map_err(|source| AppError::sqlite("open test SQLite connection", source))?;
        let transaction = connection
            .transaction()
            .map_err(|source| AppError::sqlite("start test SQLite transaction", source))?;

        transaction
            .execute_batch(
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
                CREATE TABLE node_search_validated_content (
                    node_id INTEGER PRIMARY KEY,
                    updated_at TEXT NOT NULL
                );
                ",
            )
            .map_err(|source| AppError::sqlite("create test SQLite nodes table", source))?;

        let updated_at = current_timestamp(&transaction)?;
        transaction
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
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)
                ",
                params![
                    1_i64,
                    Option::<i64>::None,
                    "Node",
                    0_i64,
                    content,
                    &updated_at,
                    &updated_at
                ],
            )
            .map_err(|source| AppError::sqlite("insert test SQLite node", source))?;
        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit test SQLite transaction", source))?;

        Ok((SqliteDocumentRepository { connection }, updated_at))
    }

    fn repository_with_sibling_orders(
        nodes: &[(i64, Option<i64>, &str, i64)],
    ) -> Result<(SqliteDocumentRepository, String), AppError> {
        let mut connection = Connection::open_in_memory()
            .map_err(|source| AppError::sqlite("open test SQLite connection", source))?;
        let transaction = connection
            .transaction()
            .map_err(|source| AppError::sqlite("start test SQLite transaction", source))?;

        transaction
            .execute_batch(
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
                CREATE TABLE node_search_validated_content (
                    node_id INTEGER PRIMARY KEY,
                    updated_at TEXT NOT NULL
                );
                ",
            )
            .map_err(|source| AppError::sqlite("create test SQLite nodes table", source))?;

        let updated_at = current_timestamp(&transaction)?;
        for (id, parent_id, title, sort_order) in nodes {
            transaction
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
                    VALUES (?1, ?2, ?3, ?4, '', ?5, ?5, NULL)
                    ",
                    params![id, parent_id, title, sort_order, &updated_at],
                )
                .map_err(|source| AppError::sqlite("insert test SQLite node", source))?;
        }
        transaction
            .commit()
            .map_err(|source| AppError::sqlite("commit test SQLite transaction", source))?;

        Ok((SqliteDocumentRepository { connection }, updated_at))
    }

    fn active_node_sort_orders(
        connection: &Connection,
        parent_id: Option<i64>,
    ) -> Result<Vec<(i64, i64)>, AppError> {
        let mut statement = connection
            .prepare(
                "
                SELECT id, sort_order
                FROM nodes
                WHERE deleted_at IS NULL
                    AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
                ORDER BY sort_order, title, id
                ",
            )
            .map_err(|source| {
                AppError::sqlite("prepare test SQLite node sort order query", source)
            })?;
        let rows = statement
            .query_map(params![parent_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|source| AppError::sqlite("query test SQLite node sort order", source))?;

        let mut sort_orders = Vec::new();
        for row in rows {
            sort_orders.push(row.map_err(|source| {
                AppError::sqlite("read test SQLite node sort order row", source)
            })?);
        }
        Ok(sort_orders)
    }

    fn active_node_updated_ats(
        connection: &Connection,
        parent_id: Option<i64>,
    ) -> Result<Vec<(i64, String)>, AppError> {
        let mut statement = connection
            .prepare(
                "
                SELECT id, updated_at
                FROM nodes
                WHERE deleted_at IS NULL
                    AND ((parent_id IS NULL AND ?1 IS NULL) OR parent_id = ?1)
                ORDER BY id
                ",
            )
            .map_err(|source| {
                AppError::sqlite("prepare test SQLite node timestamp query", source)
            })?;
        let rows = statement
            .query_map(params![parent_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|source| AppError::sqlite("query test SQLite node timestamp", source))?;

        let mut timestamps = Vec::new();
        for row in rows {
            timestamps.push(row.map_err(|source| {
                AppError::sqlite("read test SQLite node timestamp row", source)
            })?);
        }
        Ok(timestamps)
    }
}
