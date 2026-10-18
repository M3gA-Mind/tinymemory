//! One configurable model-facing name for recall, fetch, learn, and forget.
//!
//! The facade reuses the argument shaping and engine reads in this module's
//! parent. Its writes are accepted only by an injected WriteSink; the
//! PendingWriteSink adapts that port to PendingWrites. The host still owns
//! queue partitions, the active binding and gate, drain scheduling and
//! execution, settlement notifications, and the exclusive barrier that makes
//! erasure wait for an in-flight engine write.

use std::sync::Arc;

use chrono::Utc;
use serde::Serialize;
use serde_json::{Map, Value, json};
use tinymemory_api::{
    Error, FetchMode, FetchPage, FetchRequest, LearningKind, MemoryMeta, Namespace, Reach,
    RecallMode, SourceKind, SourceRef, StoreItem, ThreadWindow, ToolCallRef,
};

use crate::confine::{Confinement, Policy};
use crate::list_store::ListStore;
use crate::ops::{LearningOptions, MemoryOps};
use crate::pending::{
    Ack, Binding, Entry, PendingError, PendingWrites, WriteGate, WriteOperation,
};
use crate::tools::args::{Args, fetch_mode, meta_filter, time_hint};
use crate::tools::spec::schema::{DEFAULT_CONFIDENCE, DEFAULT_LEARNING_KIND, LEARNING_KINDS, MAX_IDS, MAX_LIMIT, default_mode};
use crate::tools::{MemoryTools, ToolScope};

const TOOL_NAME: &str = "memory";
const ACTIONS: [&str; 4] = ["recall", "fetch", "learn", "forget"];
const FETCH_TOP: usize = 5;
const EVIDENCE_TOKENS: usize = 2_500;
const PARTLY_SEARCHED: &str = "(Memory was only partly searched";
const LEARN_QUEUED: &str = "queued; not yet saved to memory";
const FORGET_QUEUED: &str = "queued; not yet removed from memory";

/// Whether an action reads memory or changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionClass {
    /// Recall and fetch only read memory.
    ReadOnly,
    /// Learn and forget change memory.
    Write,
}

/// Whether an action can safely run alongside another call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConcurrencyClass {
    /// Independent reads can run together.
    Safe,
    /// Writes must be serialized by the host tool runtime.
    Exclusive,
}

/// Per-call context that changes read behavior without changing the trusted
/// identity carried by ToolScope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallContext {
    /// The calling thread's turns still present in the prompt, when known.
    pub thread_window: Option<ThreadWindow>,
}

/// The configurable tool name, description, and one action-discriminated
/// parameter schema.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SingleToolSpec {
    /// The name a host registers for this tool.
    pub name: String,
    /// Instructions shown to the model.
    pub description: String,
    /// The closed, action-discriminated JSON parameter schema.
    pub parameters: Value,
}

/// A runtime-neutral boundary for accepting a validated memory write.
///
/// Implementations return only after the write is durable locally. They do
/// not report an engine write as complete.
pub trait WriteSink: Send + Sync {
    /// Durably queues one operation under the trusted scope.
    ///
    /// # Errors
    ///
    /// The pending store is full or unavailable.
    fn enqueue(
        &self,
        operation: WriteOperation,
        scope: &ToolScope,
    ) -> std::result::Result<Ack, PendingError>;
}

/// A host-configured adapter from tool writes to PendingWrites.
///
/// The caller supplies one queue, active owner binding, and write gate. The
/// scope root comes from trusted reach.at, or place for an unrestricted
/// single-tenant scope. This adapter does not create a drain task or provide
/// the host's exclusive in-flight erase barrier.
pub struct PendingWriteSink<S: ListStore<Entry> + 'static> {
    queue: Arc<PendingWrites<S>>,
    gate: Arc<WriteGate>,
    binding: Binding,
}

impl<S: ListStore<Entry> + 'static> std::fmt::Debug for PendingWriteSink<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingWriteSink")
            .field("queue", &self.queue)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl<S: ListStore<Entry> + 'static> PendingWriteSink<S> {
    /// Binds tool writes to the host-selected queue, erasure gate, and owner.
    #[must_use]
    pub fn new(queue: Arc<PendingWrites<S>>, gate: Arc<WriteGate>, binding: Binding) -> Self {
        Self {
            queue,
            gate,
            binding,
        }
    }
}

impl<S: ListStore<Entry> + 'static> WriteSink for PendingWriteSink<S> {
    fn enqueue(
        &self,
        operation: WriteOperation,
        scope: &ToolScope,
    ) -> std::result::Result<Ack, PendingError> {
        let scope_root = scope
            .reach
            .as_ref()
            .map(|reach| reach.at.clone())
            .unwrap_or_else(|| scope.place.clone());
        self.queue.enqueue(
            operation,
            scope_root,
            self.binding.clone(),
            &self.gate,
        )
    }
}

/// One model-facing name that dispatches its action to TinyMemory.
pub struct SingleTool {
    name: String,
    tools: MemoryTools,
    write_sink: Option<Arc<dyn WriteSink>>,
}

impl std::fmt::Debug for SingleTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SingleTool")
            .field("name", &self.name)
            .field("tools", &self.tools)
            .field("has_write_sink", &self.write_sink.is_some())
            .finish()
    }
}

impl SingleTool {
    /// One tool named name, over the engine and trusted scope in tools.
    #[must_use]
    pub fn new(name: impl Into<String>, tools: MemoryTools) -> Self {
        Self {
            name: name.into(),
            tools,
            write_sink: None,
        }
    }

    /// Installs the host's durable write port. Without one, learn and forget
    /// return an error and never fall through to a direct engine write.
    #[must_use]
    pub fn with_write_sink(mut self, sink: Arc<dyn WriteSink>) -> Self {
        self.write_sink = Some(sink);
        self
    }

    /// The single tool's name, description, and parameter schema.
    #[must_use]
    pub fn spec(&self) -> SingleToolSpec {
        SingleToolSpec {
            name: self.name.clone(),
            description: concat!(
                "Long-term memory. Set action to recall evidence for a question, fetch matching ",
                "items five at a time, learn a fact or preference, or forget items by id. ",
                "Recall before asking for information the user may already have shared. ",
                "Learn and forget responses acknowledge durable local queueing; they do not ",
                "mean a remote write has finished."
            )
            .to_string(),
            parameters: schema(self.tools.engine.descriptor().fetch_modes.as_slice()),
        }
    }

    /// Classifies an action for host permission enforcement.
    ///
    /// Unknown and missing actions default to the restrictive write class.
    #[must_use]
    pub fn permission_for(&self, args: &Value) -> PermissionClass {
        match action(args) {
            Some("recall" | "fetch") => PermissionClass::ReadOnly,
            _ => PermissionClass::Write,
        }
    }

    /// Classifies an action for host concurrency enforcement.
    ///
    /// Unknown and missing actions default to exclusive.
    #[must_use]
    pub fn concurrency_for(&self, args: &Value) -> ConcurrencyClass {
        match action(args) {
            Some("recall" | "fetch") => ConcurrencyClass::Safe,
            _ => ConcurrencyClass::Exclusive,
        }
    }

    /// Runs the action in args and returns its compact JSON result.
    ///
    /// The current thread in the trusted scope is authoritative. A matching
    /// prompt window excludes only those turns; a missing or mismatched
    /// window excludes the whole current thread.
    ///
    /// # Errors
    ///
    /// Invalid actions or arguments, reads rejected by the engine, writes
    /// rejected by the queue, and writes without an installed sink.
    pub async fn call(&self, args: Value, context: CallContext) -> tinymemory_api::Result<Value> {
        let action = action(&args)
            .ok_or_else(|| Args::invalid(TOOL_NAME, "action is required"))?;
        if !ACTIONS.contains(&action) {
            return Err(Args::invalid(
                TOOL_NAME,
                &format!("unknown action {action}; use recall, fetch, learn or forget"),
            ));
        }
        if matches!(action, "learn" | "forget") && !self.tools.scope.writes {
            return Err(Error::Unsupported(
                "memory writes are not offered in this read-only scope".into(),
            ));
        }
        match action {
            "recall" => self.recall(&args, &context).await,
            "fetch" => self.fetch(&args, &context).await,
            "learn" => self.learn(&args),
            "forget" => self.forget(&args),
            _ => Err(Args::invalid(TOOL_NAME, "unknown action")),
        }
    }

    async fn recall(
        &self,
        value: &Value,
        context: &CallContext,
    ) -> tinymemory_api::Result<Value> {
        let value = action_args(value, false)?;
        let answer = super::read::recall_answer(
            self.tools.engine.as_ref(),
            &self.tools.scope,
            &value,
            RecallMode::Evidence,
            Some(MAX_LIMIT),
            calling_thread(&self.tools.scope, context),
        )
        .await?;
        Ok(if answer.answer.trim().is_empty() {
            json!({ "evidence": "", "note": "Nothing relevant is stored in memory." })
        } else {
            json!({
                "evidence": capped_evidence(&answer.answer),
                "items": answer.citations.len()
            })
        })
    }

    async fn fetch(
        &self,
        value: &Value,
        context: &CallContext,
    ) -> tinymemory_api::Result<Value> {
        let value = action_args(value, false)?;
        let page = super::read::fetch_page(
            self.tools.engine.as_ref(),
            &self.tools.scope,
            &value,
            Some(FETCH_TOP),
            calling_thread(&self.tools.scope, context),
        )
        .await?;
        let mut result = json!({
            "matches": fetch_matches(&page),
            "items": page.hits.len()
        });
        if let Some(cursor) = page.next_cursor {
            result["next_cursor"] = json!(cursor);
        }
        Ok(result)
    }

    fn learn(&self, value: &Value) -> tinymemory_api::Result<Value> {
        let sink = self.write_sink.as_ref().ok_or_else(|| {
            Error::Unsupported("memory write queue is not configured".into())
        })?;
        let item = self.learning_item(value)?;
        match sink
            .enqueue(WriteOperation::Learn { item }, &self.tools.scope)
            .map_err(pending_error)?
        {
            Ack::Learned { id } => Ok(json!({ "id": id, "status": LEARN_QUEUED })),
            Ack::Forgotten { .. } => Err(Error::Unavailable(
                "memory queue returned an acknowledgement for the wrong operation".into(),
            )),
        }
    }

    fn forget(&self, value: &Value) -> tinymemory_api::Result<Value> {
        let sink = self.write_sink.as_ref().ok_or_else(|| {
            Error::Unsupported("memory write queue is not configured".into())
        })?;
        let value = action_args(value, false)?;
        let args = Args::parse(TOOL_NAME, &value, &["ids"])?;
        let ids = super::read::item_ids(&args, "ids")?;
        let mut ids: Vec<String> = ids.into_iter().map(|id| id.0).collect();
        ids.sort();
        ids.dedup();
        let reach = self
            .tools
            .scope
            .reach
            .clone()
            .unwrap_or_else(|| Reach::subtree(Namespace::ROOT));
        match sink
            .enqueue(WriteOperation::Forget { ids, reach }, &self.tools.scope)
            .map_err(pending_error)?
        {
            Ack::Forgotten { queued } => Ok(json!({
                "queued": queued,
                "status": FORGET_QUEUED
            })),
            Ack::Learned { .. } => Err(Error::Unavailable(
                "memory queue returned an acknowledgement for the wrong operation".into(),
            )),
        }
    }

    fn learning_item(&self, value: &Value) -> tinymemory_api::Result<StoreItem> {
        let mut value = action_args(value, true)?;
        let supplied_meta = value
            .as_object_mut()
            .and_then(|object| object.remove("meta"))
            .filter(|meta| !meta.is_null());
        if let Some(meta) = supplied_meta {
            serde_json::from_value::<MemoryMeta>(meta)
                .map_err(|_| Args::invalid(TOOL_NAME, "meta must be an object"))?;
        }
        let args = Args::parse(
            TOOL_NAME,
            &value,
            &["text", "kind", "confidence", "evidence"],
        )?;
        let kind = args
            .string("kind")?
            .map(|kind| {
                serde_json::from_value::<LearningKind>(Value::String(kind)).map_err(|_| {
                    args.field_error("kind", &format!("must be one of {}", LEARNING_KINDS.join(", ")))
                })
            })
            .transpose()?;
        let mut item = MemoryOps::new(confinement(&self.tools.scope)).learning_item(
            LearningOptions {
                text: args.required_string("text")?,
                kind,
                confidence: if args.has("confidence") {
                    Some(args.unit("confidence", DEFAULT_CONFIDENCE)?)
                } else {
                    None
                },
                meta: None,
                host_meta: Some(self.learn_meta()),
                default_namespace: self.tools.scope.place.clone(),
                now: Utc::now(),
            },
        )?;
        if let StoreItem::Learning {
            evidence: stored_evidence,
            ..
        } = &mut item
        {
            *stored_evidence = args.string("evidence")?;
        }
        Ok(item)
    }

    fn learn_meta(&self) -> MemoryMeta {
        let call_id = self
            .tools
            .scope
            .tool_call
            .as_ref()
            .and_then(|call| call.id.clone());
        MemoryMeta {
            namespace: self.tools.scope.place.clone(),
            workspace: self.tools.scope.workspace.clone(),
            thread_id: self.tools.scope.thread_id.clone(),
            agent_id: self.tools.scope.agent_id.clone(),
            tool_call: Some(ToolCallRef {
                name: self.name.clone(),
                id: call_id,
            }),
            source: SourceRef {
                kind: SourceKind::Agent,
                id: None,
            },
            ..MemoryMeta::default()
        }
    }
}

fn action(value: &Value) -> Option<&str> {
    value.get("action").and_then(Value::as_str)
}

fn action_args(value: &Value, drop_meta: bool) -> tinymemory_api::Result<Value> {
    let Value::Object(mut object) = value.clone() else {
        return Err(Args::invalid(TOOL_NAME, "arguments must be a json object"));
    };
    object.remove("action");
    if drop_meta {
        object.remove("meta");
    }
    Ok(Value::Object(object))
}

fn confinement(scope: &ToolScope) -> Confinement {
    Confinement {
        allowed: scope
            .reach
            .clone()
            .unwrap_or_else(|| Reach::subtree(Namespace::ROOT)),
        policy: Policy::Refuse,
    }
}

fn calling_thread(scope: &ToolScope, context: &CallContext) -> Option<ThreadWindow> {
    let thread_id = scope.thread_id.as_deref()?;
    Some(
        context
            .thread_window
            .as_ref()
            .filter(|window| window.thread_id == thread_id)
            .cloned()
            .unwrap_or_else(|| ThreadWindow::whole(thread_id)),
    )
}

fn fetch_matches(page: &FetchPage) -> String {
    let mut output = match page.hits.len() {
        0 => "Nothing in memory matches.".to_string(),
        1 => "1 match.".to_string(),
        count => format!("Top {count} matches, best first."),
    };
    for hit in &page.hits {
        output.push_str("\n\n");
        if let Some(at) = hit.meta.observed_at {
            output.push_str(&format!("[{}] ", at.format("%Y-%m-%d %H:%M UTC")));
        }
        match hit.kind {
            tinymemory_api::ItemKind::Learning => output.push_str("learning: "),
            tinymemory_api::ItemKind::Document => output.push_str("document: "),
            tinymemory_api::ItemKind::Conversation => {}
        }
        output.push_str(hit.text.trim());
        output.push_str(&format!("\n(id {})", hit.id));
    }
    output
}

fn capped_evidence(evidence: &str) -> String {
    const SEPARATOR: &str = "\n\n---\n\n";
    let budget = EVIDENCE_TOKENS * 4;
    if evidence.len() <= budget {
        return evidence.to_string();
    }
    let (body, closing) = match evidence.rfind(PARTLY_SEARCHED) {
        Some(at) => (&evidence[..at], &evidence[at..]),
        None => (evidence, ""),
    };
    let entries: Vec<&str> = body.split(SEPARATOR).collect();
    let mut output = String::new();
    let mut whole = 0;
    let mut cut_short = false;
    for entry in &entries {
        let joint = if output.is_empty() { "" } else { SEPARATOR };
        if output.len() + joint.len() + entry.len() <= budget {
            output.push_str(joint);
            output.push_str(entry);
            whole += 1;
            continue;
        }
        let mut lines = 0;
        for line in entry.split('\n') {
            let piece = (if lines == 0 { joint.len() } else { 1 }) + line.len();
            if output.len() + piece > budget && !output.is_empty() {
                break;
            }
            output.push_str(if lines == 0 { joint } else { "\n" });
            output.push_str(line);
            lines += 1;
        }
        if lines == entry.split('\n').count() {
            whole += 1;
        } else if lines > 0 {
            output.push_str("\n[…]");
            cut_short = true;
        }
        break;
    }
    let left = entries.len() - whole - usize::from(cut_short);
    if left > 0 || cut_short {
        let shortened = if cut_short {
            ", the last one shortened,"
        } else {
            ""
        };
        output.push_str(&format!(
            "\n\n({left} more memory entries left out{shortened} to keep this under about \
             {EVIDENCE_TOKENS} tokens; search again with more specific words to see others.)"
        ));
    }
    output.push_str(closing);
    output
}

fn pending_error(error: PendingError) -> Error {
    Error::Unavailable(error.to_string())
}

fn schema(fetch_modes: &[FetchMode]) -> Value {
    let branches = [
        branch("recall", super::spec::schema::recall()),
        branch("fetch", super::spec::schema::fetch(fetch_modes)),
        branch("learn", learn_schema()),
        branch("forget", forget_schema()),
    ];
    let mut properties = Map::new();
    properties.insert(
        "action".into(),
        json!({
            "type": "string",
            "enum": ACTIONS,
            "description": "Which memory operation to perform."
        }),
    );
    for branch in &branches {
        if let Some(branch_properties) = branch["properties"].as_object() {
            for (key, value) in branch_properties {
                if key != "action" {
                    properties
                        .entry(key.clone())
                        .or_insert_with(|| value.clone());
                }
            }
        }
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": ["action"],
        "oneOf": branches,
        "additionalProperties": false
    })
}

fn branch(action: &str, parameters: Value) -> Value {
    let mut properties = parameters["properties"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    properties.insert("action".into(), json!({"const": action}));
    let mut required = parameters["required"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    required.push(json!("action"));
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn learn_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "text": {
                "type": "string",
                "description": "The self-contained statement worth remembering."
            },
            "kind": {
                "type": "string",
                "enum": LEARNING_KINDS,
                "default": DEFAULT_LEARNING_KIND,
                "description": "What kind of statement it is."
            },
            "confidence": {
                "type": "number",
                "minimum": 0.0,
                "maximum": 1.0,
                "default": DEFAULT_CONFIDENCE
            },
            "evidence": {
                "type": "string",
                "description": "What supports the statement."
            }
        },
        "required": ["text"]
    })
}

fn forget_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "ids": {
                "type": "array",
                "items": {"type": "string"},
                "minItems": 1,
                "maxItems": MAX_IDS,
                "description": "The memory ids to remove."
            }
        },
        "required": ["ids"]
    })
}
