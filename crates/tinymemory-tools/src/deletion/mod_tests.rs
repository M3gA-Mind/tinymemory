use super::*;

use std::sync::Mutex as StdMutex;

use crate::list_store::MemoryListStore;

fn thread(id: &str) -> PendingDeletion {
    PendingDeletion::Thread {
        thread_id: id.to_string(),
    }
}

fn queue() -> DeletionQueue<MemoryListStore<PendingDeletion>> {
    DeletionQueue::new(MemoryListStore::default(), Backoff::default())
}

struct Engine {
    reachable: bool,
    failing: bool,
    ran: StdMutex<Vec<PendingDeletion>>,
}

#[async_trait]
impl DeletionExecutor for Engine {
    fn is_reachable(&self) -> bool {
        self.reachable
    }
    async fn run(&self, deletion: &PendingDeletion) -> Result<usize, String> {
        self.ran.lock().unwrap().push(deletion.clone());
        if self.failing {
            Err("DOWN".into())
        } else {
            Ok(1)
        }
    }
}

fn engine(reachable: bool, failing: bool) -> Engine {
    Engine {
        reachable,
        failing,
        ran: StdMutex::new(Vec::new()),
    }
}

#[test]
fn a_deletion_is_queued_once() {
    let queue = queue();
    queue.enqueue(thread("t"));
    queue.enqueue(thread("t"));
    queue.enqueue(PendingDeletion::Source {
        source_id: "s".into(),
    });
    assert_eq!(queue.pending().len(), 2);
}

#[tokio::test]
async fn nothing_runs_while_the_engine_is_unreachable() {
    let queue = queue();
    queue.enqueue(thread("t"));
    let engine = engine(false, false);
    assert_eq!(queue.drain(&engine, Drain::Force, Utc::now()).await, 0);
    assert!(engine.ran.lock().unwrap().is_empty());
    assert_eq!(queue.pending(), vec![thread("t")]);
}

#[tokio::test]
async fn a_reachable_engine_settles_the_queue() {
    let queue = queue();
    queue.enqueue(thread("a"));
    queue.enqueue(thread("b"));
    assert_eq!(
        queue
            .drain(&engine(true, false), Drain::Due, Utc::now())
            .await,
        2
    );
    assert!(queue.pending().is_empty());
}

#[tokio::test]
async fn a_failed_deletion_is_kept_and_backs_off_until_due() {
    let queue = queue();
    queue.enqueue(thread("t"));
    let now = Utc::now();
    let failing = engine(true, true);

    assert_eq!(queue.drain(&failing, Drain::Due, now).await, 0);
    assert_eq!(queue.pending(), vec![thread("t")]);
    assert_eq!(failing.ran.lock().unwrap().len(), 1);

    // Still backing off: a tick skips it.
    queue
        .drain(&failing, Drain::Due, now + Duration::seconds(5))
        .await;
    assert_eq!(failing.ran.lock().unwrap().len(), 1);

    // A sign-in forces it.
    queue
        .drain(&failing, Drain::Force, now + Duration::seconds(5))
        .await;
    assert_eq!(failing.ran.lock().unwrap().len(), 2);

    // Due again after the (doubled) delay, and it settles once the engine works.
    let later = now + Duration::minutes(10);
    assert_eq!(
        queue.drain(&engine(true, false), Drain::Due, later).await,
        1
    );
    assert!(queue.pending().is_empty());
}

#[test]
fn backoff_doubles_and_is_capped() {
    let backoff = Backoff {
        base: Duration::seconds(30),
        max: Duration::minutes(10),
    };
    assert_eq!(backoff.delay(1), Duration::seconds(30));
    assert_eq!(backoff.delay(2), Duration::seconds(60));
    assert_eq!(backoff.delay(3), Duration::seconds(120));
    assert_eq!(backoff.delay(50), Duration::minutes(10));
}

#[test]
fn an_unreadable_store_is_treated_as_empty_without_clobbering() {
    use crate::list_store::StoreError;
    struct Broken;
    impl ListStore<PendingDeletion> for Broken {
        fn load(&self) -> Result<Vec<PendingDeletion>, StoreError> {
            Err(StoreError("io".into()))
        }
        fn save(&self, _: &[PendingDeletion]) -> Result<(), StoreError> {
            Ok(())
        }
    }
    let queue = DeletionQueue::new(Broken, Backoff::default());
    assert!(queue.pending().is_empty());
}

#[test]
fn the_wire_shape_matches_the_legacy_file() {
    let json = serde_json::to_string(&thread("t")).unwrap();
    assert_eq!(json, r#"{"kind":"thread","thread_id":"t"}"#);
}
