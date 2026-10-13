//! Where an item from the legacy tree goes in the per-user tree.
//!
//! The rule: an item goes where today's write path would put it now, so a
//! later sync of the same item replays instead of storing a duplicate.
//!
//! - **Brain documents** at `source:<id>` go to W4's connector node:
//!   [`legacy_brain_node`], then [`brain_node_with`] and [`file_into`]: the
//!   same placement new syncs use (`pdf`, `markdown` and the other
//!   per-format nodes become `files`; GitHub splits by repository when the
//!   setting is on).
//! - **Conversations** are pooled at the chat node; the agent id stays in
//!   the item's metadata.
//! - **Workflow items** (keyed memory and run digests, tagged `flow:<id>`)
//!   stay with the root's learnings and documents for now, as they are read
//!   today. [`FlowPlacement::InNode`] is the one switch that moves them into
//!   each flow's own node, once flow memory is read from there.
//! - Everything else keeps its namespace; only the wire root changes.

use tinymemory_api::{ItemKind, Namespace, SegmentKind, StoreItem};
use tinymemory_tools::MemoryLayout;

use crate::memory::brain::{brain_node_with, file_into, legacy_brain_node};

/// The tag every workflow item carries. Legacy items carry it whatever
/// features the host build has, so placing them must not depend on them.
pub const FLOWS_TAG: &str = "flows";

/// Where workflow items go.
#[derive(Clone, Copy)]
pub enum FlowPlacement {
    /// With the root's learnings and documents, as today.
    WithRoot,
    /// In each flow's own node: the flow id to its node.
    InNode(fn(&str) -> Result<Namespace>),
}

/// Where items go in the per-user tree.
pub struct Placement {
    /// The layout new items are written with.
    pub layout: MemoryLayout,
    /// Where conversations are pooled.
    pub chat_node: Namespace,
    /// Where workflow items go.
    pub flows: FlowPlacement,
    /// `[memory] split_github_by_repo`: GitHub documents go one collection
    /// per repository, as a sync files them now.
    pub split_github_by_repo: bool,
}

/// The brain source of local files and uploads, of any format.
#[must_use]
pub fn files_source() -> BrainSource {
    BrainSource::Other("files".to_string())
}

/// Canonical form of a connector slug found on a legacy brain node
/// (`google_drive` becomes `googledrive`), so documents filed by the removed
/// Composio sync still migrate to the node they were filed under.
fn legacy_connector_slug(slug: &str) -> String {
    let key = slug.trim().to_ascii_lowercase();
    match key.as_str() {
        "feishu" | "lark" => "larksuite".to_string(),
        "google_calendar" => "googlecalendar".to_string(),
        "google_drive" => "googledrive".to_string(),
        "google_sheets" => "googlesheets".to_string(),
        _ => key,
    }
}

/// The brain source a document filed under the old per-type layout belongs
/// to now: `old_source_id` is the id of the `source:<id>` node it sits at,
/// `item` the document itself.
///
/// - Per-format nodes (`pdf`, `markdown`, `docx`, `xlsx`, `pptx`, `code`,
///   `other`) held local files: `files`.
/// - `web` held links and feeds, but also HTML files from a folder or an
///   upload; a document with a file kind or a file path is a file.
/// - Any other node was a connector: its canonical slug (`google_drive`
///   becomes `googledrive`), so `notion`, `github` and `gmail` stay put.
#[must_use]
pub fn legacy_brain_node(old_source_id: &str, item: &StoreItem) -> BrainSource {
    let id = legacy_connector_slug(old_source_id);
    match id.as_str() {
        "files" | "pdf" | "markdown" | "md" | "docx" | "xlsx" | "pptx" | "code" | "other" => {
            files_source()
        }
        "web" => {
            let meta = item.meta();
            let file = matches!(meta.source.kind, SourceKind::File | SourceKind::Folder)
                || meta.file_path.is_some();
            if file {
                files_source()
            } else {
                BrainSource::Web
            }
        }
        _ => id.parse().unwrap_or_else(|_| files_source()),
    }
}

/// The repository a GitHub document belongs to, as the collection id
/// `<owner>--<repo>` (lowercase, as GitHub's names are case-insensitive):
/// from its `repo` (`owner/name` or a URL), else its URL. An owner name
/// never holds `--` nor ends in `-`, so the first `--` always splits the
/// two and no two repositories share an id (`foo-bar/repo` and
/// `foo/bar-repo` stay apart).
#[must_use]
pub fn github_collection(item: &StoreItem) -> Option<String> {
    let meta = item.meta();
    [meta.repo.as_deref(), meta.url.as_deref()]
        .into_iter()
        .flatten()
        .find_map(|raw| {
            let path = raw
                .trim()
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches("www.")
                .trim_start_matches("github.com/");
            // A URL's query or fragment is not part of the path.
            let path = path.split(['?', '#']).next().unwrap_or_default();
            let mut parts = path.split('/').filter(|part| !part.is_empty());
            let (owner, repo) = (parts.next()?, parts.next()?);
            let repo = repo.trim_end_matches(".git");
            (!owner.contains('.') && !repo.is_empty())
                .then(|| format!("{owner}--{repo}").to_ascii_lowercase())
        })
}

/// The node `item` of `source` is filed at: the source's node, or, for
/// GitHub with `split_github_by_repo` on, its repository's collection below
/// it.
///
/// # Errors
///
/// When the node cannot be built (an id the namespace grammar refuses).
pub fn brain_node_with(
    split_github_by_repo: bool,
    layout: &MemoryLayout,
    source: &BrainSource,
    item: &StoreItem,
) -> Result<Namespace> {
    if split_github_by_repo && *source == BrainSource::Github {
        if let Some(repo) = github_collection(item) {
            return Ok(layout.brain_collection(source, &repo)?);
        }
    }
    Ok(layout.brain(source)?)
}

/// `item` placed in the brain at `node`, with no agent id (the brain
/// belongs to every agent).
#[must_use]
pub fn file_into(node: Namespace, mut item: StoreItem) -> StoreItem {
    let meta = item.meta_mut();
    meta.namespace = node;
    meta.agent_id = None;
    item
}

/// The flow a workflow item belongs to: its `flow:<id>` tag, when it also
/// carries the `flows` tag (keyed memory and run digests both do).
fn flow_of(item: &StoreItem) -> Option<&str> {
    let tags = &item.meta().tags;
    if !tags.iter().any(|tag| tag == FLOWS_TAG) {
        return None;
    }
    tags.iter()
        .filter_map(|tag| tag.strip_prefix("flow:"))
        .find(|id| !id.contains(':'))
}

/// The id of the `source:` segment directly below `root`, if `namespace` is
/// a brain node of the legacy layout.
fn legacy_source<'a>(root: &Namespace, namespace: &'a Namespace) -> Option<&'a str> {
    let (root, segments) = (root.segments(), namespace.segments());
    let below = segments.get(root.len())?;
    (segments.starts_with(root) && below.kind() == SegmentKind::Source).then(|| below.id())
}

impl Placement {
    /// `item` as it is stored in the per-user tree.
    ///
    /// # Errors
    ///
    /// When a node cannot be built (an id the namespace grammar refuses).
    pub fn place(&self, mut item: StoreItem) -> Result<StoreItem> {
        if let Some(flow) = flow_of(&item) {
            if let FlowPlacement::InNode(node) = self.flows {
                item.meta_mut().namespace = node(flow)?;
            }
            return Ok(item);
        }
        if item.kind() == ItemKind::Conversation {
            item.meta_mut().namespace = self.chat_node.clone();
            return Ok(item);
        }
        if item.kind() == ItemKind::Document {
            let namespace = item.meta().namespace.clone();
            if let Some(id) = legacy_source(self.layout.root(), &namespace) {
                let source = legacy_brain_node(id, &item);
                let node =
                    brain_node_with(self.split_github_by_repo, &self.layout, &source, &item)?;
                return Ok(file_into(node, item));
            }
        }
        Ok(item)
    }
}

#[cfg(test)]
#[path = "map_tests.rs"]
mod tests;
