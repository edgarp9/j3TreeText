use super::DomainError;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::fmt;

pub const ROOT_NODE_ID: i64 = 1;
pub const DEFAULT_DOCUMENT_ID: i64 = 2;
pub const ROOT_TITLE: &str = "Root";
pub const DEFAULT_DOCUMENT_TITLE: &str = "Untitled";
pub const SEARCH_RESULT_LIMIT: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SiblingMoveDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub title: String,
    pub sort_order: i64,
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSiblingOrderUpdate {
    pub node_id: i64,
    pub parent_id: Option<i64>,
    pub sort_order: i64,
    pub updated_at: String,
}

impl Node {
    pub fn root_document(created_at: String, updated_at: String) -> Self {
        Self {
            id: ROOT_NODE_ID,
            parent_id: None,
            title: ROOT_TITLE.to_owned(),
            sort_order: 0,
            content: String::new(),
            created_at,
            updated_at,
            deleted_at: None,
        }
    }

    pub fn default_document(created_at: String, updated_at: String) -> Self {
        Self {
            id: DEFAULT_DOCUMENT_ID,
            parent_id: Some(ROOT_NODE_ID),
            title: DEFAULT_DOCUMENT_TITLE.to_owned(),
            sort_order: 0,
            content: String::new(),
            created_at,
            updated_at,
            deleted_at: None,
        }
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        if self.id <= 0 {
            return Err(DomainError::InvalidNodeId(self.id));
        }

        if let Some(parent_id) = self.parent_id {
            if parent_id <= 0 || parent_id == self.id {
                return Err(DomainError::InvalidParent {
                    node_id: self.id,
                    parent_id,
                });
            }
        }

        if self.title.trim().is_empty() {
            return Err(DomainError::EmptyTitle { node_id: self.id });
        }

        if self.title.contains('\0') {
            return Err(DomainError::EmbeddedNulTitle { node_id: self.id });
        }

        if self.content.contains('\0') {
            return Err(DomainError::EmbeddedNulContent { node_id: self.id });
        }

        if self.sort_order < 0 {
            return Err(DomainError::InvalidSortOrder {
                node_id: self.id,
                sort_order: self.sort_order,
            });
        }

        if self.created_at.trim().is_empty() || self.updated_at.trim().is_empty() {
            return Err(DomainError::MissingTimestamp { node_id: self.id });
        }

        Ok(())
    }
}

type NodeIndicesById = HashMap<i64, usize>;
type SiblingTitleOwners = HashMap<Option<i64>, HashMap<String, Vec<i64>>>;

struct RemovedNodeMetadata {
    index: usize,
    id: i64,
    parent_id: Option<i64>,
    title: String,
}

#[derive(Clone)]
pub struct Document {
    nodes: Vec<Node>,
    node_indices_by_id: NodeIndicesById,
    sibling_title_owners: SiblingTitleOwners,
}

impl fmt::Debug for Document {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Document")
            .field("nodes", &self.nodes)
            .finish()
    }
}

impl PartialEq for Document {
    fn eq(&self, other: &Self) -> bool {
        self.nodes == other.nodes
    }
}

impl Eq for Document {}

fn add_sibling_title_owner(
    owners_by_parent: &mut SiblingTitleOwners,
    parent_id: Option<i64>,
    title: String,
    node_id: i64,
) {
    owners_by_parent
        .entry(parent_id)
        .or_default()
        .entry(title)
        .or_default()
        .push(node_id);
}

fn has_sibling_title_owner_except(
    owners_by_parent: &SiblingTitleOwners,
    parent_id: Option<i64>,
    title: &str,
    excluded_node_id: Option<i64>,
) -> bool {
    owners_by_parent
        .get(&parent_id)
        .and_then(|owners_by_title| owners_by_title.get(title))
        .is_some_and(|owners| {
            owners
                .iter()
                .any(|owner_id| Some(*owner_id) != excluded_node_id)
        })
}

impl Document {
    pub fn new(nodes: Vec<Node>) -> Result<Self, DomainError> {
        for node in &nodes {
            node.validate()?;
        }

        let mut node_indices_by_id = NodeIndicesById::with_capacity(nodes.len());
        for (index, node) in nodes.iter().enumerate() {
            match node_indices_by_id.entry(node.id) {
                Entry::Vacant(entry) => {
                    entry.insert(index);
                }
                Entry::Occupied(_) => {
                    return Err(DomainError::DuplicateNodeId(node.id));
                }
            }
        }

        let mut sibling_title_owners = SiblingTitleOwners::with_capacity(nodes.len());
        for node in &nodes {
            if has_sibling_title_owner_except(
                &sibling_title_owners,
                node.parent_id,
                &node.title,
                None,
            ) {
                return Err(DomainError::DuplicateSiblingTitle {
                    parent_id: node.parent_id,
                    title: node.title.clone(),
                });
            }
            add_sibling_title_owner(
                &mut sibling_title_owners,
                node.parent_id,
                node.title.clone(),
                node.id,
            );
        }

        let mut root_count = 0;
        let mut root_id = None;
        for node in &nodes {
            if node.parent_id.is_none() {
                root_count += 1;
                if root_id.is_none() {
                    root_id = Some(node.id);
                }
            }
        }
        if root_count == 0 {
            return Err(DomainError::MissingRoot);
        }
        if root_count > 1 {
            return Err(DomainError::MultipleRoots);
        }

        for node in &nodes {
            if let Some(parent_id) = node.parent_id {
                if !node_indices_by_id.contains_key(&parent_id) {
                    return Err(DomainError::MissingParent {
                        node_id: node.id,
                        parent_id,
                    });
                }
            }
        }

        let root_id = root_id.ok_or(DomainError::MissingRoot)?;
        validate_tree_reaches_root(&nodes, root_id, &node_indices_by_id)?;

        Ok(Self {
            nodes,
            node_indices_by_id,
            sibling_title_owners,
        })
    }

    pub fn node_by_id(&self, node_id: i64) -> Option<&Node> {
        self.node_indices_by_id
            .get(&node_id)
            .map(|index| &self.nodes[*index])
    }

    pub fn insert_node(&mut self, node: Node) -> Result<(), DomainError> {
        self.insert_nodes(vec![node])
    }

    pub fn insert_nodes(&mut self, nodes: Vec<Node>) -> Result<(), DomainError> {
        if nodes.is_empty() {
            return Ok(());
        }

        let mut incoming_ids = HashSet::with_capacity(nodes.len());
        for node in &nodes {
            node.validate()?;
            if self.node_indices_by_id.contains_key(&node.id) || !incoming_ids.insert(node.id) {
                return Err(DomainError::DuplicateNodeId(node.id));
            }
        }

        let incoming_parent_by_id: HashMap<i64, Option<i64>> =
            nodes.iter().map(|node| (node.id, node.parent_id)).collect();
        let existing_has_root = self.nodes.iter().any(|node| node.parent_id.is_none());
        let incoming_root_count = nodes.iter().filter(|node| node.parent_id.is_none()).count();
        if existing_has_root && incoming_root_count > 0 || incoming_root_count > 1 {
            return Err(DomainError::MultipleRoots);
        }

        for node in &nodes {
            if let Some(parent_id) = node.parent_id {
                if !self.node_indices_by_id.contains_key(&parent_id)
                    && !incoming_ids.contains(&parent_id)
                {
                    return Err(DomainError::MissingParent {
                        node_id: node.id,
                        parent_id,
                    });
                }
            }
            validate_incoming_parent_path(
                node.id,
                &incoming_parent_by_id,
                &incoming_ids,
                |parent_id| self.node_indices_by_id.contains_key(&parent_id),
            )?;
        }

        self.validate_sibling_titles_with_inserted_nodes(&nodes)?;
        let start_index = self.nodes.len();
        self.nodes.extend(nodes);
        for index in start_index..self.nodes.len() {
            let node_id = self.nodes[index].id;
            let parent_id = self.nodes[index].parent_id;
            let title = self.nodes[index].title.clone();
            self.node_indices_by_id.insert(node_id, index);
            add_sibling_title_owner(&mut self.sibling_title_owners, parent_id, title, node_id);
        }
        Ok(())
    }

    pub fn rename_node(
        &mut self,
        node_id: i64,
        title: String,
        updated_at: String,
    ) -> Result<(), DomainError> {
        if title.trim().is_empty() {
            return Err(DomainError::EmptyTitle { node_id });
        }
        if title.contains('\0') {
            return Err(DomainError::EmbeddedNulTitle { node_id });
        }
        if updated_at.trim().is_empty() {
            return Err(DomainError::MissingTimestamp { node_id });
        }

        let index = self
            .node_index_by_id(node_id)
            .ok_or(DomainError::NodeNotFound { node_id })?;
        let parent_id = self.nodes[index].parent_id;
        self.validate_unique_sibling_title(parent_id, &title, Some(node_id))?;

        if self.nodes[index].title != title {
            let old_title = self.nodes[index].title.clone();
            self.nodes[index].title = title;
            self.remove_sibling_title_owner(parent_id, &old_title, node_id);
            let new_title = self.nodes[index].title.clone();
            add_sibling_title_owner(
                &mut self.sibling_title_owners,
                parent_id,
                new_title,
                node_id,
            );
        }
        self.nodes[index].updated_at = updated_at;
        Ok(())
    }

    pub fn replace_node_content(
        &mut self,
        node_id: i64,
        content: String,
        updated_at: String,
    ) -> Result<(), DomainError> {
        if content.contains('\0') {
            return Err(DomainError::EmbeddedNulContent { node_id });
        }
        if updated_at.trim().is_empty() {
            return Err(DomainError::MissingTimestamp { node_id });
        }

        let index = self
            .node_index_by_id(node_id)
            .ok_or(DomainError::NodeNotFound { node_id })?;
        self.nodes[index].content = content;
        self.nodes[index].updated_at = updated_at;
        Ok(())
    }

    pub fn remove_nodes_and_apply_sibling_order_updates(
        &mut self,
        node_ids: &[i64],
        updates: &[NodeSiblingOrderUpdate],
    ) -> Result<(), DomainError> {
        if node_ids.is_empty() && updates.is_empty() {
            return Ok(());
        }

        let mut removed_ids = HashSet::with_capacity(node_ids.len());
        for node_id in node_ids {
            if *node_id == ROOT_NODE_ID {
                return Err(DomainError::CannotDeleteRoot);
            }
            if !self.node_indices_by_id.contains_key(node_id) {
                return Err(DomainError::NodeNotFound { node_id: *node_id });
            }
            removed_ids.insert(*node_id);
        }

        self.validate_sibling_order_updates(updates, Some(&removed_ids), &self.node_indices_by_id)?;
        self.apply_sibling_order_update_values(updates);
        if !removed_ids.is_empty() {
            self.remove_nodes_by_id_preserving_metadata(&removed_ids);
        }
        Ok(())
    }

    pub fn apply_sibling_order_updates(
        &mut self,
        updates: &[NodeSiblingOrderUpdate],
    ) -> Result<(), DomainError> {
        if updates.is_empty() {
            return Ok(());
        }

        let indexed_updates =
            self.validate_existing_sibling_order_updates(updates, &self.node_indices_by_id)?;
        self.validate_sibling_order_update_result(&indexed_updates, &self.node_indices_by_id)?;
        self.apply_indexed_sibling_order_updates(&indexed_updates);
        Ok(())
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    #[cfg(test)]
    pub fn root(&self) -> Option<&Node> {
        self.nodes.iter().find(|node| node.parent_id.is_none())
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    fn node_index_by_id(&self, node_id: i64) -> Option<usize> {
        self.node_indices_by_id.get(&node_id).copied()
    }

    fn validate_sibling_titles_with_inserted_nodes(
        &self,
        inserted_nodes: &[Node],
    ) -> Result<(), DomainError> {
        let mut inserted_titles = HashSet::with_capacity(inserted_nodes.len());
        for node in inserted_nodes {
            if has_sibling_title_owner_except(
                &self.sibling_title_owners,
                node.parent_id,
                &node.title,
                None,
            ) || !inserted_titles.insert((node.parent_id, node.title.as_str()))
            {
                return Err(DomainError::DuplicateSiblingTitle {
                    parent_id: node.parent_id,
                    title: node.title.clone(),
                });
            }
        }
        Ok(())
    }

    fn validate_unique_sibling_title(
        &self,
        parent_id: Option<i64>,
        title: &str,
        excluded_node_id: Option<i64>,
    ) -> Result<(), DomainError> {
        if has_sibling_title_owner_except(
            &self.sibling_title_owners,
            parent_id,
            title,
            excluded_node_id,
        ) {
            return Err(DomainError::DuplicateSiblingTitle {
                parent_id,
                title: title.to_owned(),
            });
        }
        Ok(())
    }

    fn validate_sibling_order_updates(
        &self,
        updates: &[NodeSiblingOrderUpdate],
        removed_ids: Option<&HashSet<i64>>,
        node_indices_by_id: &NodeIndicesById,
    ) -> Result<(), DomainError> {
        let mut update_ids = HashSet::with_capacity(updates.len());
        for update in updates {
            if !update_ids.insert(update.node_id) {
                return Err(DomainError::DuplicateNodeId(update.node_id));
            }
            if update.sort_order < 0 {
                return Err(DomainError::InvalidSortOrder {
                    node_id: update.node_id,
                    sort_order: update.sort_order,
                });
            }
            if update.updated_at.trim().is_empty() {
                return Err(DomainError::MissingTimestamp {
                    node_id: update.node_id,
                });
            }
            if removed_ids.is_some_and(|ids| ids.contains(&update.node_id))
                || !node_indices_by_id.contains_key(&update.node_id)
            {
                return Err(DomainError::NodeNotFound {
                    node_id: update.node_id,
                });
            }
            if let Some(parent_id) = update.parent_id {
                if parent_id <= 0 || parent_id == update.node_id {
                    return Err(DomainError::InvalidParent {
                        node_id: update.node_id,
                        parent_id,
                    });
                }
                if removed_ids.is_some_and(|ids| ids.contains(&parent_id))
                    || !node_indices_by_id.contains_key(&parent_id)
                {
                    return Err(DomainError::MissingParent {
                        node_id: update.node_id,
                        parent_id,
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_existing_sibling_order_updates<'a>(
        &self,
        updates: &'a [NodeSiblingOrderUpdate],
        node_indices_by_id: &NodeIndicesById,
    ) -> Result<Vec<(usize, &'a NodeSiblingOrderUpdate)>, DomainError> {
        let mut update_ids = HashSet::with_capacity(updates.len());
        let mut indexed_updates = Vec::with_capacity(updates.len());
        for update in updates {
            if !update_ids.insert(update.node_id) {
                return Err(DomainError::DuplicateNodeId(update.node_id));
            }
            if update.sort_order < 0 {
                return Err(DomainError::InvalidSortOrder {
                    node_id: update.node_id,
                    sort_order: update.sort_order,
                });
            }
            if update.updated_at.trim().is_empty() {
                return Err(DomainError::MissingTimestamp {
                    node_id: update.node_id,
                });
            }

            let index = node_indices_by_id.get(&update.node_id).copied().ok_or(
                DomainError::NodeNotFound {
                    node_id: update.node_id,
                },
            )?;

            if let Some(parent_id) = update.parent_id {
                if parent_id <= 0 || parent_id == update.node_id {
                    return Err(DomainError::InvalidParent {
                        node_id: update.node_id,
                        parent_id,
                    });
                }
                if !node_indices_by_id.contains_key(&parent_id) {
                    return Err(DomainError::MissingParent {
                        node_id: update.node_id,
                        parent_id,
                    });
                }
            }

            indexed_updates.push((index, update));
        }
        Ok(indexed_updates)
    }

    fn validate_sibling_order_update_result(
        &self,
        indexed_updates: &[(usize, &NodeSiblingOrderUpdate)],
        node_indices_by_id: &NodeIndicesById,
    ) -> Result<(), DomainError> {
        let mut parent_updates = Vec::new();
        let mut updates_by_id = HashMap::with_capacity(indexed_updates.len());
        for (index, update) in indexed_updates {
            if self.nodes[*index].parent_id != update.parent_id {
                parent_updates.push((*index, *update));
            }
            updates_by_id.insert(update.node_id, *update);
        }

        if parent_updates.is_empty() {
            return Ok(());
        }

        self.validate_sibling_titles_after_parent_updates(&updates_by_id, &parent_updates)?;
        self.validate_root_count_after_parent_updates(&parent_updates)?;
        self.validate_parent_paths_after_parent_updates(
            &updates_by_id,
            &parent_updates,
            node_indices_by_id,
        )
    }

    fn validate_sibling_titles_after_parent_updates(
        &self,
        updates_by_id: &HashMap<i64, &NodeSiblingOrderUpdate>,
        parent_updates: &[(usize, &NodeSiblingOrderUpdate)],
    ) -> Result<(), DomainError> {
        let mut affected_parents = HashSet::with_capacity(parent_updates.len() * 2);
        let mut incoming_titles_by_parent: HashMap<Option<i64>, HashMap<&str, i64>> =
            HashMap::with_capacity(parent_updates.len());
        for (index, update) in parent_updates {
            let node = &self.nodes[*index];
            let title = node.title.as_str();
            affected_parents.insert(node.parent_id);
            affected_parents.insert(update.parent_id);

            let owner_by_title = incoming_titles_by_parent
                .entry(update.parent_id)
                .or_default();
            if owner_by_title
                .insert(title, node.id)
                .is_some_and(|existing_node_id| existing_node_id != node.id)
            {
                return Err(DomainError::DuplicateSiblingTitle {
                    parent_id: update.parent_id,
                    title: node.title.clone(),
                });
            }
        }

        for parent_id in affected_parents {
            let Some(owners_by_title) = self.sibling_title_owners.get(&parent_id) else {
                continue;
            };
            for (title, owners) in owners_by_title {
                let remaining_count = owners
                    .iter()
                    .filter(|owner_id| match updates_by_id.get(owner_id) {
                        Some(update) => update.parent_id == parent_id,
                        None => true,
                    })
                    .count();
                let incoming_count = incoming_titles_by_parent
                    .get(&parent_id)
                    .and_then(|owners_by_title| owners_by_title.get(title.as_str()))
                    .map_or(0, |_| 1);
                if remaining_count + incoming_count > 1 {
                    return Err(DomainError::DuplicateSiblingTitle {
                        parent_id,
                        title: title.clone(),
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_root_count_after_parent_updates(
        &self,
        parent_updates: &[(usize, &NodeSiblingOrderUpdate)],
    ) -> Result<(), DomainError> {
        let mut root_count = 1;
        for (index, update) in parent_updates {
            match (
                self.nodes[*index].parent_id.is_none(),
                update.parent_id.is_none(),
            ) {
                (true, false) => root_count -= 1,
                (false, true) => root_count += 1,
                _ => {}
            }
        }

        if root_count == 0 {
            return Err(DomainError::MissingRoot);
        }
        if root_count > 1 {
            return Err(DomainError::MultipleRoots);
        }
        Ok(())
    }

    fn validate_parent_paths_after_parent_updates(
        &self,
        updates_by_id: &HashMap<i64, &NodeSiblingOrderUpdate>,
        parent_updates: &[(usize, &NodeSiblingOrderUpdate)],
        node_indices_by_id: &NodeIndicesById,
    ) -> Result<(), DomainError> {
        for (index, _) in parent_updates {
            self.validate_projected_parent_path(
                self.nodes[*index].id,
                updates_by_id,
                node_indices_by_id,
            )?;
        }
        Ok(())
    }

    fn validate_projected_parent_path(
        &self,
        node_id: i64,
        updates_by_id: &HashMap<i64, &NodeSiblingOrderUpdate>,
        node_indices_by_id: &NodeIndicesById,
    ) -> Result<(), DomainError> {
        let mut seen = HashSet::new();
        let mut current_id = node_id;

        loop {
            if !seen.insert(current_id) {
                return Err(DomainError::ParentCycle {
                    node_id: current_id,
                });
            }

            let Some(parent_id) =
                self.projected_parent_id(current_id, updates_by_id, node_indices_by_id)?
            else {
                return Ok(());
            };
            current_id = parent_id;
        }
    }

    fn projected_parent_id(
        &self,
        node_id: i64,
        updates_by_id: &HashMap<i64, &NodeSiblingOrderUpdate>,
        node_indices_by_id: &NodeIndicesById,
    ) -> Result<Option<i64>, DomainError> {
        if let Some(update) = updates_by_id.get(&node_id) {
            return Ok(update.parent_id);
        }
        let index = node_indices_by_id
            .get(&node_id)
            .copied()
            .ok_or(DomainError::NodeNotFound { node_id })?;
        Ok(self.nodes[index].parent_id)
    }

    fn apply_indexed_sibling_order_updates(
        &mut self,
        indexed_updates: &[(usize, &NodeSiblingOrderUpdate)],
    ) {
        for (index, update) in indexed_updates {
            let node_id = self.nodes[*index].id;
            let old_parent_id = self.nodes[*index].parent_id;
            if old_parent_id != update.parent_id {
                let title = self.nodes[*index].title.clone();
                self.remove_sibling_title_owner(old_parent_id, &title, node_id);
                add_sibling_title_owner(
                    &mut self.sibling_title_owners,
                    update.parent_id,
                    title,
                    node_id,
                );
            }

            let node = &mut self.nodes[*index];
            node.parent_id = update.parent_id;
            node.sort_order = update.sort_order;
            node.updated_at = update.updated_at.clone();
        }
    }

    fn apply_sibling_order_update_values(&mut self, updates: &[NodeSiblingOrderUpdate]) {
        if updates.is_empty() {
            return;
        }

        for update in updates {
            let Some(index) = self.node_indices_by_id.get(&update.node_id).copied() else {
                continue;
            };

            let old_parent_id = self.nodes[index].parent_id;
            if old_parent_id != update.parent_id {
                let title = self.nodes[index].title.clone();
                self.remove_sibling_title_owner(old_parent_id, &title, update.node_id);
                add_sibling_title_owner(
                    &mut self.sibling_title_owners,
                    update.parent_id,
                    title,
                    update.node_id,
                );
            }

            let node = &mut self.nodes[index];
            node.parent_id = update.parent_id;
            node.sort_order = update.sort_order;
            node.updated_at = update.updated_at.clone();
        }
    }

    fn remove_sibling_title_owner(&mut self, parent_id: Option<i64>, title: &str, node_id: i64) {
        let Some(owners_by_title) = self.sibling_title_owners.get_mut(&parent_id) else {
            return;
        };

        let mut remove_title = false;
        if let Some(owners) = owners_by_title.get_mut(title) {
            if let Some(index) = owners.iter().position(|owner_id| *owner_id == node_id) {
                owners.swap_remove(index);
            }
            remove_title = owners.is_empty();
        }
        if remove_title {
            owners_by_title.remove(title);
        }
        if owners_by_title.is_empty() {
            self.sibling_title_owners.remove(&parent_id);
        }
    }

    fn remove_nodes_by_id_preserving_metadata(&mut self, removed_ids: &HashSet<i64>) {
        let mut removed_nodes = Vec::with_capacity(removed_ids.len());
        for node_id in removed_ids {
            let Some(index) = self.node_indices_by_id.get(node_id).copied() else {
                continue;
            };
            let Some(node) = self.nodes.get(index) else {
                continue;
            };
            removed_nodes.push(RemovedNodeMetadata {
                index,
                id: node.id,
                parent_id: node.parent_id,
                title: node.title.clone(),
            });
        }
        if removed_nodes.is_empty() {
            return;
        }

        let mut first_changed_index = self.nodes.len();
        let mut indexed_shift_cost = 0usize;
        for removed in &removed_nodes {
            first_changed_index = first_changed_index.min(removed.index);
            indexed_shift_cost = indexed_shift_cost
                .saturating_add(self.nodes.len().saturating_sub(removed.index + 1));
            self.node_indices_by_id.remove(&removed.id);
        }
        for removed in &removed_nodes {
            self.remove_sibling_title_owner(removed.parent_id, &removed.title, removed.id);
        }

        if indexed_shift_cost <= self.nodes.len() {
            removed_nodes.sort_unstable_by_key(|removed| std::cmp::Reverse(removed.index));
            for removed in removed_nodes {
                if self
                    .nodes
                    .get(removed.index)
                    .is_some_and(|node| node.id == removed.id)
                {
                    self.nodes.remove(removed.index);
                } else if let Some(index) = self.nodes.iter().position(|node| node.id == removed.id)
                {
                    first_changed_index = first_changed_index.min(index);
                    self.nodes.remove(index);
                }
            }
        } else {
            self.nodes.retain(|node| !removed_ids.contains(&node.id));
        }

        self.refresh_node_indices_from(first_changed_index);
    }

    fn refresh_node_indices_from(&mut self, start_index: usize) {
        for (index, node) in self.nodes.iter().enumerate().skip(start_index) {
            self.node_indices_by_id.insert(node.id, index);
        }
    }
}

fn validate_incoming_parent_path(
    node_id: i64,
    incoming_parent_by_id: &HashMap<i64, Option<i64>>,
    incoming_ids: &HashSet<i64>,
    existing_node_exists: impl Fn(i64) -> bool,
) -> Result<(), DomainError> {
    let mut seen = HashSet::new();
    let mut current_id = node_id;

    while let Some(parent_id) = incoming_parent_by_id.get(&current_id).copied().flatten() {
        if existing_node_exists(parent_id) {
            return Ok(());
        }
        if !incoming_ids.contains(&parent_id) {
            return Err(DomainError::MissingParent { node_id, parent_id });
        }
        if !seen.insert(current_id) {
            return Err(DomainError::ParentCycle {
                node_id: current_id,
            });
        }
        current_id = parent_id;
    }

    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReachabilityState {
    Unknown,
    Visiting,
    ReachesRoot,
}

fn validate_tree_reaches_root(
    nodes: &[Node],
    root_id: i64,
    node_indices_by_id: &NodeIndicesById,
) -> Result<(), DomainError> {
    let Some(root_index) = node_indices_by_id.get(&root_id).copied() else {
        return Err(DomainError::MissingRoot);
    };

    let mut reachability = vec![ReachabilityState::Unknown; nodes.len()];
    reachability[root_index] = ReachabilityState::ReachesRoot;
    let mut path = Vec::new();

    for start_index in 0..nodes.len() {
        if reachability[start_index] == ReachabilityState::ReachesRoot {
            continue;
        }

        path.clear();
        let mut current_index = start_index;

        loop {
            match reachability[current_index] {
                ReachabilityState::ReachesRoot => {
                    for &path_index in &path {
                        reachability[path_index] = ReachabilityState::ReachesRoot;
                    }
                    break;
                }
                ReachabilityState::Visiting => {
                    return Err(DomainError::ParentCycle {
                        node_id: nodes[current_index].id,
                    });
                }
                ReachabilityState::Unknown => {
                    reachability[current_index] = ReachabilityState::Visiting;
                    path.push(current_index);
                }
            }

            let current_node = &nodes[current_index];
            let Some(parent_id) = current_node.parent_id else {
                return Err(DomainError::UnreachableNode {
                    node_id: nodes[start_index].id,
                });
            };
            let Some(parent_index) = node_indices_by_id.get(&parent_id).copied() else {
                return Err(DomainError::MissingParent {
                    node_id: nodes[start_index].id,
                    parent_id,
                });
            };
            current_index = parent_index;
        }
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSearchResult {
    pub node: Node,
    pub parent_title: Option<String>,
    pub content_matched: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i64, parent_id: Option<i64>, title: &str, sort_order: i64) -> Node {
        Node {
            id,
            parent_id,
            title: title.to_owned(),
            sort_order,
            content: String::new(),
            created_at: "2026-05-22T00:00:00Z".to_owned(),
            updated_at: "2026-05-22T00:00:00Z".to_owned(),
            deleted_at: None,
        }
    }

    fn sibling_update(
        node_id: i64,
        parent_id: Option<i64>,
        sort_order: i64,
    ) -> NodeSiblingOrderUpdate {
        NodeSiblingOrderUpdate {
            node_id,
            parent_id,
            sort_order,
            updated_at: "2026-05-22T00:00:01Z".to_owned(),
        }
    }

    #[test]
    fn rename_node_updates_sibling_title_metadata() -> Result<(), DomainError> {
        let mut document = Document::new(vec![
            node(ROOT_NODE_ID, None, ROOT_TITLE, 0),
            node(2, Some(ROOT_NODE_ID), "Alpha", 0),
            node(3, Some(ROOT_NODE_ID), "Beta", 1),
        ])?;

        document.rename_node(2, "Gamma".to_owned(), "2026-05-22T00:00:02Z".to_owned())?;
        document.rename_node(3, "Alpha".to_owned(), "2026-05-22T00:00:03Z".to_owned())?;

        assert_eq!(
            Some("Alpha"),
            document.node_by_id(3).map(|node| node.title.as_str())
        );
        assert!(matches!(
            document.rename_node(2, "Alpha".to_owned(), "2026-05-22T00:00:04Z".to_owned()),
            Err(DomainError::DuplicateSiblingTitle {
                parent_id: Some(ROOT_NODE_ID),
                title,
            }) if title == "Alpha"
        ));
        Ok(())
    }

    #[test]
    fn sibling_order_updates_validate_titles_with_cached_metadata() -> Result<(), DomainError> {
        let mut document = Document::new(vec![
            node(ROOT_NODE_ID, None, ROOT_TITLE, 0),
            node(2, Some(ROOT_NODE_ID), "Left", 0),
            node(3, Some(ROOT_NODE_ID), "Right", 1),
            node(4, Some(2), "Leaf", 0),
            node(5, Some(3), "Leaf", 0),
        ])?;

        document.apply_sibling_order_updates(&[
            sibling_update(4, Some(3), 0),
            sibling_update(5, Some(2), 0),
        ])?;

        assert_eq!(
            Some(3),
            document.node_by_id(4).and_then(|node| node.parent_id)
        );
        assert_eq!(
            Some(2),
            document.node_by_id(5).and_then(|node| node.parent_id)
        );
        assert!(matches!(
            document.apply_sibling_order_updates(&[sibling_update(4, Some(2), 1)]),
            Err(DomainError::DuplicateSiblingTitle {
                parent_id: Some(2),
                title,
            }) if title == "Leaf"
        ));
        Ok(())
    }

    #[test]
    fn remove_nodes_updates_cached_metadata_for_remaining_nodes() -> Result<(), DomainError> {
        let mut document = Document::new(vec![
            node(ROOT_NODE_ID, None, ROOT_TITLE, 0),
            node(2, Some(ROOT_NODE_ID), "Alpha", 0),
            node(3, Some(ROOT_NODE_ID), "Beta", 1),
            node(4, Some(ROOT_NODE_ID), "Gamma", 2),
        ])?;

        document.remove_nodes_and_apply_sibling_order_updates(
            &[2],
            &[sibling_update(3, Some(ROOT_NODE_ID), 0)],
        )?;

        assert!(document.node_by_id(2).is_none());
        assert_eq!(document.node_by_id(3).map(|node| node.sort_order), Some(0));
        document.rename_node(3, "Alpha".to_owned(), "2026-05-22T00:00:02Z".to_owned())?;
        assert_eq!(
            document.node_by_id(3).map(|node| node.title.as_str()),
            Some("Alpha")
        );
        assert!(matches!(
            document.rename_node(4, "Alpha".to_owned(), "2026-05-22T00:00:03Z".to_owned()),
            Err(DomainError::DuplicateSiblingTitle {
                parent_id: Some(ROOT_NODE_ID),
                title,
            }) if title == "Alpha"
        ));
        Ok(())
    }
}
