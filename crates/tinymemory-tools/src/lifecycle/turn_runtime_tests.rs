//! Deterministic port tests for turn orchestration and cancellation.

use std::future::{Future, pending};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::{BoxFuture, poll_fn};
use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{
    EngineDescriptor, EngineHealth, Error, FetchPage, FetchRequest, ForgetReport, ForgetTarget,
    Hit, ItemKind, ListPage, ListRequest, MemoryEngine, MetaFilter, RecallAnswer, RecallRequest,
    Result, StoreItem, StoreReceipt, Turn,
};
use tinymemory_tools::background::BackgroundJob;
use tinymemory_tools::brain::{Brain, BrainDocument};
use tinymemory_tools::layout::{BrainSource, MemoryLayout};
use tinymemory_tools::prefetch::{Limits, Prefetcher, PrefetchRuntime};

use super::turn::{
    CitationSink, Clock, CompactionOptions, DateHintModel, EngineResolver, JobsSink, PreTurnOptions,
    TurnHooks, TurnPack,
};
use super::{AgentMemory, Compaction, PostTurn, PreTurn, RecallPolicy};

#[derive(Clone)]
struct FakeClock {
    now: Duration,
    immediate: bool,
    deadlines: Arc<Mutex<Vec<Duration>>>,
}

impl FakeClock {
    fn pending_at(now: Duration) -> Self {
        Self {
            now,
            immediate: false,
            deadlines: Arc::default(),
        }
    }

    fn immediate_at(now: Duration) -> Self {
        Self {
            now,
            immediate: true,
            deadlines: Arc::default(),
        }
    }

    fn deadlines(&self) -> Vec<Duration> {
        self.deadlines.lock().unwrap().clone()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        self.now
    }

    fn sleep_until(&self, deadline: Duration) -> BoxFuture<'static, ()> {
        self.deadlines.lock().unwrap().push(deadline);
        if self.immediate {
            Box::pin(async {})
        } else {
            Box::pin(pending())
        }
    }
}

struct StaticResolver {
    calls: AtomicUsize,
    memory: AgentMemory,
    fail: bool,
}

impl EngineResolver for StaticResolver {
    fn resolve(&self) -> Result<AgentMemory> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            Err(Error::Unavailable("resolver offline".into()))
        } else {
            Ok(self.memory.clone())
        }
    }
}

#[derive(Default)]
struct SpyModel {
    calls: AtomicUsize,
    input: Mutex<Vec<(String, String, String)>>,
    answer: Option<String>,
    held: bool,
    dropped: Arc<AtomicBool>,
}

impl DateHintModel for SpyModel {
    fn complete(
        &self,
        instruction: String,
        user_text: String,
        zone: String,
    ) -> BoxFuture<'static, Option<String>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.input
            .lock()
            .unwrap()
            .push((instruction, user_text, zone));
        let answer = self.answer.clone();
        let dropped = self.dropped.clone();
        let held = self.held;
        Box::pin(async move {
            struct MarkDropped(Arc<AtomicBool>);
            impl Drop for MarkDropped {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::SeqCst);
                }
            }
            let _guard = MarkDropped(dropped);
            if held {
                pending::<()>().await;
            }
            answer
        })
    }
}

#[derive(Default)]
struct RecordingJobs {
    calls: AtomicUsize,
    jobs: Mutex<Vec<BackgroundJob>>,
}

#[async_trait]
impl JobsSink for RecordingJobs {
    async fn enqueue(&self, jobs: Vec<BackgroundJob>) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.jobs.lock().unwrap().extend(jobs);
    }
}

#[derive(Default)]
struct RecordingCitations {
    calls: AtomicUsize,
    hits: Mutex<Vec<Hit>>,
}

impl CitationSink for RecordingCitations {
    fn deliver(&self, citations: Vec<Hit>) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.hits.lock().unwrap().extend(citations);
    }
}

fn memory(engine: Arc<dyn MemoryEngine>) -> AgentMemory {
    AgentMemory::new(engine, MemoryLayout::default(), "agent")
        .unwrap()
        .with_policy(RecallPolicy::default())
}

struct Fixture {
    hooks: Arc<TurnHooks>,
    resolver: Arc<StaticResolver>,
    clock: FakeClock,
    model: Arc<SpyModel>,
    jobs: Arc<RecordingJobs>,
    citations: Arc<RecordingCitations>,
    engine: Option<Arc<ReferenceEngine>>,
}

fn fixture(clock: FakeClock, fail_resolve: bool) -> Fixture {
    let engine = Arc::new(ReferenceEngine::new());
    fixture_with_engine(clock, fail_resolve, engine.clone(), Some(engine))
}

fn fixture_with_engine(
    clock: FakeClock,
    fail_resolve: bool,
    engine: Arc<dyn MemoryEngine>,
    reference: Option<Arc<ReferenceEngine>>,
) -> Fixture {
    let resolver = Arc::new(StaticResolver {
        calls: AtomicUsize::new(0),
        memory: memory(engine),
        fail: fail_resolve,
    });
    let model = Arc::new(SpyModel::default());
    let jobs = Arc::new(RecordingJobs::default());
    let citations = Arc::new(RecordingCitations::default());
    let hooks = Arc::new(TurnHooks::new(
        resolver.clone(),
        Arc::new(clock.clone()),
        Some(model.clone()),
        jobs.clone(),
        citations.clone(),
    ));
    Fixture {
        hooks,
        resolver,
        clock,
        model,
        jobs,
        citations,
        engine: reference,
    }
}

fn options(logging: bool, recall: bool) -> PreTurnOptions {
    PreTurnOptions {
        logging,
        recall,
        timeout: Duration::from_secs(1),
        date_hint: true,
        zone: Some("Asia/Kolkata".into()),
        date_hint_margin: Duration::from_millis(100),
    }
}

async fn stored_conversations(engine: &ReferenceEngine) -> usize {
    engine.list(ListRequest::new(
        MetaFilter::kinds([ItemKind::Conversation]),
        100,
    ))
    .await
    .unwrap()
    .items
    .len()
}

#[tokio::test]
async fn pre_turn_obeys_all_logging_and_recall_combinations() {
    for (logging, recall, resolves, logs, model_calls) in [
        (false, false, false, 0, 0),
        (false, true, true, 0, 0),
        (true, false, true, 1, 0),
        (true, true, true, 1, 1),
    ] {
        let fixture = fixture(FakeClock::pending_at(Duration::from_secs(20)), false);
        let _ = fixture
            .hooks
            .pre_turn(PreTurn::new("thread", 0, "what happened?"), options(logging, recall))
            .await;
        assert_eq!(fixture.resolver.calls.load(Ordering::SeqCst), resolves);
        assert_eq!(
            stored_conversations(fixture.engine.as_ref().unwrap()).await,
            logs
        );
        assert_eq!(fixture.model.calls.load(Ordering::SeqCst), model_calls);
    }
}

#[tokio::test]
async fn pre_turn_resumed_flag_and_legacy_overrides_have_defined_precedence() {
    let engine = Arc::new(ReferenceEngine::new());
    let memory = memory(engine.clone());
    memory
        .pre_turn(PreTurn::new("thread", 0, "The old project name was Cedar."))
        .await
        .unwrap();
    memory
        .post_turn(PostTurn::new("thread", 1, "I will remember Cedar."))
        .await
        .unwrap();

    let resumed = PreTurn {
        resumed: true,
        in_prompt_from: 2,
        ..PreTurn::new("thread", 2, "what was the old project name?")
    };
    let ordinary = memory.pre_turn(resumed.clone()).await.unwrap().pack;
    assert!(ordinary.markdown.contains("Earlier in this thread"));
    assert!(ordinary.markdown.contains("Cedar"));

    let mut explicit_plain = resumed.clone();
    explicit_plain.resumed = false;
    let forced_resumed = memory
        .pre_turn_resumed(explicit_plain.clone())
        .await
        .unwrap()
        .pack;
    assert!(forced_resumed.markdown.contains("Earlier in this thread"));

    let forced_plain = memory
        .pre_turn_dated(explicit_plain, false, async { None })
        .await
        .unwrap()
        .pack;
    assert!(!forced_plain.markdown.contains("Earlier in this thread"));
}

#[tokio::test]
async fn resolver_failure_degrades_to_no_pack() {
    let fixture = fixture(FakeClock::pending_at(Duration::ZERO), true);
    let pack = fixture
        .hooks
        .pre_turn(PreTurn::new("thread", 0, "hello"), options(true, true))
        .await;
    assert!(pack.is_none());
    assert_eq!(fixture.resolver.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn account_wide_pre_turn_refusal_is_a_notice_not_an_empty_pack() {
    let engine: Arc<dyn MemoryEngine> = Arc::new(HeldEngine {
        inner: ReferenceEngine::new(),
        held_reads: false,
        fail_reads: true,
        fetch_dropped: Arc::new(AtomicBool::new(false)),
        recall_dropped: Arc::new(AtomicBool::new(false)),
    });
    let fixture = fixture_with_engine(
        FakeClock::pending_at(Duration::ZERO),
        false,
        engine,
        None,
    );
    let pack = fixture
        .hooks
        .pre_turn(PreTurn::new("thread", 0, "did you save it?"), options(true, true))
        .await
        .expect("account-wide failures must remain distinguishable");
    assert_eq!(pack.refusal.as_deref(), Some("INSUFFICIENT_CREDITS"));
    assert!(pack.markdown.contains("does not mean nothing is stored"));
    assert!(pack.citations.is_empty());
}

#[tokio::test]
async fn date_hint_uses_the_turn_deadline_and_the_margin_from_that_deadline() {
    let fixture = fixture(FakeClock::pending_at(Duration::from_secs(5)), false);
    let mut options = options(true, true);
    options.timeout = Duration::from_secs(1);
    fixture
        .hooks
        .pre_turn(PreTurn::new("thread", 0, "yesterday"), options)
        .await;
    assert_eq!(
        fixture.clock.deadlines(),
        [Duration::from_secs(6), Duration::from_millis(5_900)]
    );
    let calls = fixture.model.input.lock().unwrap();
    assert!(calls[0].0.contains("YYYY-MM-DD"));
    assert_eq!(calls[0].1, "yesterday");
    assert_eq!(calls[0].2, "Asia/Kolkata");
}

struct DropSignal(Arc<AtomicBool>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct HeldEngine {
    inner: ReferenceEngine,
    held_reads: bool,
    fail_reads: bool,
    fetch_dropped: Arc<AtomicBool>,
    recall_dropped: Arc<AtomicBool>,
}

#[async_trait]
impl MemoryEngine for HeldEngine {
    fn descriptor(&self) -> &EngineDescriptor {
        self.inner.descriptor()
    }

    async fn health(&self) -> EngineHealth {
        self.inner.health().await
    }

    async fn recall(&self, request: RecallRequest) -> Result<RecallAnswer> {
        if self.fail_reads {
            return Err(Error::InsufficientCredits("quota exhausted".into()));
        }
        if self.held_reads {
            let _guard = DropSignal(self.recall_dropped.clone());
            pending::<()>().await;
        }
        self.inner.recall(request).await
    }

    async fn fetch(&self, request: FetchRequest) -> Result<FetchPage> {
        if self.fail_reads {
            return Err(Error::InsufficientCredits("quota exhausted".into()));
        }
        if self.held_reads {
            let _guard = DropSignal(self.fetch_dropped.clone());
            pending::<()>().await;
        }
        self.inner.fetch(request).await
    }

    async fn store(&self, item: StoreItem) -> Result<StoreReceipt> {
        self.inner.store(item).await
    }

    async fn forget(&self, target: ForgetTarget) -> Result<ForgetReport> {
        self.inner.forget(target).await
    }

    async fn list(&self, request: ListRequest) -> Result<ListPage> {
        self.inner.list(request).await
    }
}

fn held_fixture(clock: FakeClock) -> (Fixture, Arc<AtomicBool>, Arc<AtomicBool>) {
    let fetch_dropped = Arc::new(AtomicBool::new(false));
    let recall_dropped = Arc::new(AtomicBool::new(false));
    let engine: Arc<dyn MemoryEngine> = Arc::new(HeldEngine {
        inner: ReferenceEngine::new(),
        held_reads: true,
        fail_reads: false,
        fetch_dropped: fetch_dropped.clone(),
        recall_dropped: recall_dropped.clone(),
    });
    (
        fixture_with_engine(clock, false, engine, None),
        fetch_dropped,
        recall_dropped,
    )
}

#[tokio::test]
async fn timeout_cancels_in_flight_engine_and_model_work() {
    let (fixture, fetch_dropped, recall_dropped) =
        held_fixture(FakeClock::immediate_at(Duration::from_secs(10)));
    fixture.model.held = true;
    let model_dropped = fixture.model.dropped.clone();
    let mut options = options(true, true);
    options.timeout = Duration::from_secs(1);
    assert!(fixture
        .hooks
        .pre_turn(PreTurn::new("thread", 0, "yesterday"), options)
        .await
        .is_none());
    assert!(fetch_dropped.load(Ordering::SeqCst));
    assert!(model_dropped.load(Ordering::SeqCst));
    let _ = recall_dropped;
}

#[tokio::test]
async fn dropping_owned_turn_future_cancels_in_flight_engine_and_model_work() {
    let (fixture, fetch_dropped, _) = held_fixture(FakeClock::pending_at(Duration::ZERO));
    fixture.model.held = true;
    let model_dropped = fixture.model.dropped.clone();
    let mut work = Box::pin(
        fixture
            .hooks
            .pre_turn(PreTurn::new("thread", 0, "yesterday"), options(true, true)),
    );
    poll_fn(|cx| match work.as_mut().poll(cx) {
        Poll::Pending => Poll::Ready(()),
        Poll::Ready(_) => panic!("held pre-turn unexpectedly completed"),
    })
    .await;
    assert!(!fetch_dropped.load(Ordering::SeqCst));
    assert!(!model_dropped.load(Ordering::SeqCst));
    drop(work);
    assert!(fetch_dropped.load(Ordering::SeqCst));
    assert!(model_dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn post_turn_enqueues_returned_jobs_once_and_skips_engine_when_logging_is_off() {
    let fixture = fixture(FakeClock::pending_at(Duration::ZERO), false);
    fixture
        .hooks
        .post_turn(PostTurn::new("thread", 9, "done"), true)
        .await;
    assert_eq!(fixture.resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.jobs.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.jobs.jobs.lock().unwrap().len(), 1);

    let disabled = fixture(FakeClock::pending_at(Duration::ZERO), false);
    disabled
        .hooks
        .post_turn(PostTurn::new("thread", 9, "done"), false)
        .await;
    assert_eq!(disabled.resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(disabled.jobs.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn compaction_gates_recall_and_degrades_on_timeout_or_engine_error() {
    let gated = fixture(FakeClock::pending_at(Duration::ZERO), false);
    let empty = Compaction {
        thread_id: "thread".into(),
        dropped: vec![],
        focus: None,
    };
    assert!(gated
        .hooks
        .compaction(empty.clone(), CompactionOptions::new(false, Duration::from_secs(1)))
        .await
        .is_none());
    assert_eq!(gated.resolver.calls.load(Ordering::SeqCst), 0);

    let blank = fixture(FakeClock::pending_at(Duration::ZERO), false);
    assert!(blank
        .hooks
        .compaction(
            Compaction {
                thread_id: "  ".into(),
                dropped: vec![Turn::new(tinymemory_api::Role::User, "old")],
                focus: None,
            },
            CompactionOptions::new(true, Duration::from_secs(1)),
        )
        .await
        .is_none());
    assert_eq!(blank.resolver.calls.load(Ordering::SeqCst), 0);

    let (timed, _, recall_dropped) = held_fixture(FakeClock::immediate_at(Duration::ZERO));
    let request = Compaction {
        thread_id: "thread".into(),
        dropped: vec![Turn::new(tinymemory_api::Role::User, "old"), Turn::new(tinymemory_api::Role::Assistant, "reply")],
        focus: None,
    };
    assert!(timed
        .hooks
        .compaction(request, CompactionOptions::new(true, Duration::from_secs(1)))
        .await
        .is_none());
    assert!(recall_dropped.load(Ordering::SeqCst));

    let failing_engine: Arc<dyn MemoryEngine> = Arc::new(HeldEngine {
        inner: ReferenceEngine::new(),
        held_reads: false,
        fail_reads: true,
        fetch_dropped: Arc::new(AtomicBool::new(false)),
        recall_dropped: Arc::new(AtomicBool::new(false)),
    });
    let failing = fixture_with_engine(
        FakeClock::pending_at(Duration::ZERO),
        false,
        failing_engine,
        None,
    );
    // Compaction errors remain a no-pack degradation; they do not turn into
    // pre-turn refusal notices.
    assert!(failing
        .hooks
        .compaction(
            Compaction {
                thread_id: "thread".into(),
                dropped: vec![Turn::new(tinymemory_api::Role::User, "old")],
                focus: None,
            },
            CompactionOptions::new(true, Duration::from_secs(1)),
        )
        .await
        .is_none());
}

#[tokio::test]
async fn citations_are_built_from_pack_refs_and_delivered_only_explicitly() {
    let fixture = fixture(FakeClock::pending_at(Duration::ZERO), false);
    let engine = fixture.engine.as_ref().unwrap();
    Brain::new(engine.clone(), MemoryLayout::default())
        .ingest(BrainDocument::new(
            BrainSource::Markdown,
            "The office closes on Fridays.",
        ))
        .await
        .unwrap();
    let pack = fixture
        .hooks
        .pre_turn(PreTurn::new("thread", 0, "when does the office close?"), options(true, true))
        .await
        .expect("nonempty recall pack");
    assert!(!pack.citations.is_empty());
    assert!(pack
        .citations
        .iter()
        .all(|hit| pack.refs.contains(&hit.id.to_string())));
    assert_eq!(fixture.citations.calls.load(Ordering::SeqCst), 0);

    fixture.hooks.deliver_citations(&pack);
    assert_eq!(fixture.citations.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.citations.hits.lock().unwrap().len(),
        pack.citations.len()
    );
}

struct TokioPrefetch;

impl PrefetchRuntime for TokioPrefetch {
    fn spawn(&self, task: BoxFuture<'static, ()>) {
        tokio::spawn(task);
    }

    fn sleep(&self, _duration: Duration) -> BoxFuture<'static, ()> {
        Box::pin(pending())
    }
}

#[tokio::test]
async fn owned_hook_work_runs_in_existing_prefetcher_without_delivering_citations() {
    let fixture = fixture(FakeClock::pending_at(Duration::ZERO), false);
    Brain::new(
        fixture.engine.as_ref().unwrap().clone(),
        MemoryLayout::default(),
    )
    .ingest(BrainDocument::new(
        BrainSource::Markdown,
        "The project started in 2024.",
    ))
    .await
    .unwrap();
    let cache: Prefetcher<&'static str, TurnPack> = Prefetcher::new(Arc::new(TokioPrefetch))
        .with_limits(Limits {
            work_timeout: Duration::from_secs(1),
            ..Limits::default()
        });
    cache.enqueue(
        "thread",
        fixture.hooks.pre_turn_work(
            PreTurn::new("thread", 0, "what is stored?"),
            options(true, true),
        ),
    );
    for _ in 0..100 {
        if cache.is_idle(&"thread") {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(cache.is_idle(&"thread"));
    assert!(cache.completed(&"thread").is_some());
    assert_eq!(fixture.citations.calls.load(Ordering::SeqCst), 0);
}
