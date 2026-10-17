//! Regression tests for owner-scoped mutation barriers.

use super::*;
use std::sync::Arc;

use crate::list_store::MemoryListStore;

struct SuccessfulExecutor;

#[async_trait::async_trait]
impl WriteExecutor for SuccessfulExecutor {
    async fn run(&self, _operation: &WriteOperation) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn owner_registry_releases_idle_owners_and_recreates_a_fresh_epoch() {
    let registry = OwnerRegistry::new();
    let owner = registry.get("workspace/user-a");
    let first_process = owner.gate().process().to_owned();

    assert!(Arc::ptr_eq(&owner, &registry.get("workspace/user-a")));
    assert!(!first_process.is_empty());
    drop(owner);

    let replacement = registry.get("workspace/user-b");
    drop(replacement);
    let recreated = registry.get("workspace/user-a");

    assert_ne!(recreated.gate().process(), first_process);
    assert_eq!(registry.tracked_owners(), 1);
}

#[tokio::test]
async fn overlapping_fences_share_one_gate_and_retire_earlier_mutations() {
    let registry = OwnerRegistry::new();
    let owner = registry.get("workspace/user-a");
    let mutation = owner.automatic_mutation();
    let first_fence = owner.begin_fence().await;
    let second_owner = registry.get("workspace/user-a");
    assert!(Arc::ptr_eq(&owner, &second_owner));

    let result = mutation.run(async { 7 }).await;
    assert_eq!(result, None, "a mutation admitted before erasure is stale");

    let mut second_fence = Box::pin(owner.begin_fence());
    assert!(futures::poll!(second_fence.as_mut()).is_pending());
    drop(first_fence);
    let second_fence = second_fence.await;
    assert!(owner.gate().is_fenced());
    drop(second_fence);
    assert!(!owner.gate().is_fenced());
}

#[tokio::test]
async fn mutations_wait_for_the_fence_and_resume_after_it_drops() {
    let registry = OwnerRegistry::new();
    let owner = registry.get("workspace/user-a");
    let fence = owner.begin_fence().await;
    let mutation = owner.automatic_mutation();
    let mut running = Box::pin(mutation.run(async { "ran" }));
    assert!(futures::poll!(running.as_mut()).is_pending());

    drop(fence);
    assert_eq!(running.await, Some("ran"));
}

#[tokio::test]
async fn cancelling_a_waiting_owner_fence_releases_its_gate_token() {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::task::{Context, Poll};

    struct HoldMutation(Arc<AtomicBool>);

    impl Future for HoldMutation {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            self.0.store(true, Ordering::SeqCst);
            Poll::Pending
        }
    }

    let owner = OwnerRegistry::new().get("workspace/user-a");
    let entered = Arc::new(AtomicBool::new(false));
    let mutation = owner
        .automatic_mutation()
        .run(HoldMutation(entered.clone()));
    let mut mutation = Box::pin(mutation);
    assert!(futures::poll!(mutation.as_mut()).is_pending());
    assert!(entered.load(Ordering::SeqCst));

    let mut fence = Box::pin(owner.begin_fence());
    assert!(futures::poll!(fence.as_mut()).is_pending());
    assert!(owner.gate().is_fenced());
    drop(fence);
    assert!(!owner.gate().is_fenced());
    drop(mutation);

    let completed = owner.begin_fence().await;
    assert!(owner.gate().is_fenced());
    drop(completed);
}

#[test]
fn active_mutations_and_fences_keep_the_owner_identity_alive() {
    let registry = OwnerRegistry::new();
    let owner = registry.get("workspace/user-a");
    let first_process = owner.gate().process().to_owned();
    let mutation = owner.automatic_mutation();
    drop(owner);
    let retained = registry.get("workspace/user-a");
    assert_eq!(retained.gate().process(), first_process);
    drop(mutation);
    drop(retained);

    let owner = registry.get("workspace/user-a");
    let fence_process = owner.gate().process().to_owned();
    let fence = owner.begin_fence().await;
    drop(owner);
    let retained = registry.get("workspace/user-a");
    assert_eq!(retained.gate().process(), fence_process);
    drop(fence);
    drop(retained);

    assert_ne!(registry.get("workspace/user-a").gate().process(), fence_process);
}

#[tokio::test]
async fn a_recreated_owner_fence_clears_old_persisted_epochs_before_drain() {
    use tinymemory_api::Namespace;

    let registry = OwnerRegistry::new();
    let queue = PendingWrites::new(MemoryListStore::<Entry>::default(), Limits::default());
    let old_owner = registry.get("workspace/user-a");
    let old_process = old_owner.gate().process().to_owned();
    queue
        .enqueue(
            WriteOperation::Learn {
                item: tinymemory_api::StoreItem::learning(
                    "persisted before owner eviction",
                    tinymemory_api::LearningKind::Fact,
                    0.8,
                    tinymemory_api::MemoryMeta::default(),
                ),
            },
            Namespace::ROOT,
            Binding("owner-a".into()),
            old_owner.gate(),
        )
        .unwrap();
    drop(old_owner);

    let owner = registry.get("workspace/user-a");
    assert_ne!(owner.gate().process(), old_process);
    let fence = owner.begin_fence().await;
    assert_eq!(
        queue
            .clear_stale(fence.gate().process(), fence.generation())
            .unwrap(),
        1
    );
    let executor = SuccessfulExecutor;
    assert_eq!(
        queue
            .drain(&executor, fence.gate(), fence.generation(), |_| true, |_| {})
            .await,
        0
    );
    drop(fence);
}
