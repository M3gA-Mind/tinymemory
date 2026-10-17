//! Weakly cached async locks that serialize drains per durable partition.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, Weak};

use async_lock::Mutex as AsyncMutex;

type Locks = HashMap<String, Weak<AsyncMutex<()>>>;

static GLOBAL_LOCKS: LazyLock<PartitionLocks> = LazyLock::new(PartitionLocks::new);

/// Weak registry of drain locks, with one independent lock per storage key.
///
/// A host normally keys this registry by the path or identifier of each
/// durable queue. Locks disappear as soon as the last drain drops its handle;
/// dead keys are pruned on the next lookup.
#[derive(Debug)]
pub struct PartitionLocks {
    locks: Mutex<Locks>,
}

impl Default for PartitionLocks {
    fn default() -> Self {
        Self::new()
    }
}

impl PartitionLocks {
    /// Creates an empty registry of partition drain locks.
    #[must_use]
    pub fn new() -> Self {
        Self {
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the process-wide partition lock registry.
    #[must_use]
    pub fn global() -> &'static Self {
        &GLOBAL_LOCKS
    }

    /// Gets the shared drain lock for one durable partition.
    #[must_use]
    pub fn for_partition(&self, key: impl Into<String>) -> Arc<AsyncMutex<()>> {
        let key = key.into();
        let mut locks = self
            .locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
            return lock;
        }
        let lock = Arc::new(AsyncMutex::new(()));
        locks.insert(key, Arc::downgrade(&lock));
        lock
    }

    #[cfg(test)]
    pub(super) fn tracked_partitions(&self) -> usize {
        self.locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}
