//! Public binder and timing behavior at the integration boundary.

use std::sync::{Arc, Mutex, Once};

use async_trait::async_trait;
use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{
    EngineDescriptor, EngineHealth, FetchPage, FetchRequest, ForgetReport, ForgetTarget, ListPage,
    ListRequest, MemoryEngine, RecallAnswer, RecallRequest, Result, StoreItem, StoreReceipt,
};
use tinymemory_integrations::{EngineBinder, safety::Policy};

static RECORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Capture;

impl log::Log for Capture {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        RECORDS
            .lock()
            .expect("capture lock")
            .push(record.args().to_string());
    }

    fn flush(&self) {}
}

static CAPTURE: Capture = Capture;

fn install_capture() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        log::set_logger(&CAPTURE).expect("integration test has a dedicated logger");
        log::set_max_level(log::LevelFilter::Trace);
    });
}

struct FailingEngine(ReferenceEngine);

#[async_trait]
impl MemoryEngine for FailingEngine {
    fn descriptor(&self) -> &EngineDescriptor {
        self.0.descriptor()
    }

    async fn health(&self) -> EngineHealth {
        self.0.health().await
    }

    async fn recall(&self, _: RecallRequest) -> Result<RecallAnswer> {
        Err(tinymemory_api::Error::Unauthorized(
            "private backend detail".to_string(),
        ))
    }

    async fn fetch(&self, req: FetchRequest) -> Result<FetchPage> {
        self.0.fetch(req).await
    }

    async fn store(&self, item: StoreItem) -> Result<StoreReceipt> {
        self.0.store(item).await
    }

    async fn forget(&self, target: ForgetTarget) -> Result<ForgetReport> {
        self.0.forget(target).await
    }

    async fn list(&self, req: ListRequest) -> Result<ListPage> {
        self.0.list(req).await
    }
}

#[tokio::test]
async fn external_binding_logs_one_content_free_timing_line_on_failure() {
    install_capture();
    RECORDS.lock().expect("capture lock").clear();

    let binder = EngineBinder::default();
    let bound = binder.bind_external(
        Arc::new(FailingEngine(ReferenceEngine::new())),
        "profile://transient",
        Policy::default(),
    );
    let failure = bound
        .engine
        .recall(RecallRequest::new("private user prompt", 5))
        .await
        .expect_err("the probe fails deliberately");
    assert_eq!(failure.code(), "UNAUTHORIZED");

    let lines: Vec<_> = RECORDS
        .lock()
        .expect("capture lock")
        .iter()
        .filter(|line| line.contains("[memory:timing]"))
        .cloned()
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("op=recall"), "{}", lines[0]);
    assert!(lines[0].contains("engine=reference"), "{}", lines[0]);
    assert!(lines[0].contains("elapsed_ms="), "{}", lines[0]);
    assert!(lines[0].contains("outcome=UNAUTHORIZED"), "{}", lines[0]);
    assert!(!lines[0].contains("private user prompt"), "{}", lines[0]);
    assert!(!lines[0].contains("private backend detail"), "{}", lines[0]);
}
