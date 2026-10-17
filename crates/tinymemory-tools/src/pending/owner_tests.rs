//! Regression tests for owner-scoped mutation barriers.

use super::*;
use std::sync::Arc;

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

    let fence_owner = owner.clone();
    let second_fence = tokio::spawn(async move { fence_owner.begin_fence().await });
    tokio::task::yield_now().await;
    assert!(
        !second_fence.is_finished(),
        "fences serialize on the owner barrier"
    );
    drop(first_fence);
    let second_fence = second_fence.await.unwrap();
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
    let running = tokio::spawn(async move { mutation.run(async { "ran" }).await });
    tokio::task::yield_now().await;
    assert!(!running.is_finished());

    drop(fence);
    assert_eq!(running.await.unwrap(), Some("ran"));
}
