//! Regression tests for per-partition drain serialization.

use super::*;
use std::sync::Arc;

#[tokio::test]
async fn partition_locks_serialize_one_file_without_blocking_another() {
    let locks = PartitionLocks::new();
    let first = locks.for_partition("owner-a/root");
    let same = locks.for_partition("owner-a/root");
    let other = locks.for_partition("owner-a/agent");
    assert!(Arc::ptr_eq(&first, &same));
    assert!(!Arc::ptr_eq(&first, &other));

    let held = first.lock_arc().await;
    let independent = other.try_lock_arc();
    assert!(independent.is_some(), "a different file has an independent drain lock");
    assert!(same.try_lock_arc().is_none(), "one file has only one drain at a time");
    drop(independent);
    drop(held);
}

#[tokio::test]
async fn cancelling_a_waiting_partition_drain_releases_its_lock_request() {
    let locks = PartitionLocks::new();
    let partition = locks.for_partition("owner-a/root");
    let held = partition.lock_arc().await;
    let waiting_partition = partition.clone();
    let waiting = tokio::spawn(async move { waiting_partition.lock_arc().await });
    tokio::task::yield_now().await;
    assert!(!waiting.is_finished());

    waiting.abort();
    let _ = waiting.await;
    drop(held);

    assert!(partition.try_lock_arc().is_some());
}

#[test]
fn partition_registry_evicts_locks_after_the_last_handle_drops() {
    let locks = PartitionLocks::new();
    let first = locks.for_partition("owner-a/root");
    drop(first);

    let replacement = locks.for_partition("owner-a/agent");

    assert_eq!(locks.tracked_partitions(), 1);
    assert!(replacement.try_lock_arc().is_some());
}
