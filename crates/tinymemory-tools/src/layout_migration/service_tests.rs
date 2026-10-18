use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use tinymemory_api::MemoryEngine;

use super::super::job::{Trigger, run};
use super::super::copy::Engines;
use super::super::job::{LayoutHost, LegacyClaim};
use super::super::map::Placement;
use super::super::state::{MigrationState, MigrationStateStore};
use super::super::test_host::{FakeHost, count, fact};
use super::*;

async fn move_all(host: &FakeHost) {
    run(host, Trigger::Auto, || async { false }).await.unwrap();
}

#[tokio::test]
async fn scan_finds_legacy_memory_and_nothing_after_the_move() {
    let host = FakeHost::with(3).await;
    assert_eq!(
        scan(&host).await.unwrap(),
        ScanView {
            needed: true,
            shared: false
        }
    );
    move_all(&host).await;
    assert!(!scan(&host).await.unwrap().needed);
    assert_eq!(status(&host, false).unwrap().state.phase, Phase::Cleaned);
}

#[tokio::test]
async fn nothing_to_move_shows_nothing() {
    let host = FakeHost::with(0).await;
    assert!(!scan(&host).await.unwrap().needed);
}

#[tokio::test]
async fn a_stopped_run_reads_as_interrupted_until_one_is_going() {
    let host = FakeHost::with(120).await;
    let mut stopped = host.store.load().unwrap();
    stopped.phase = Phase::Copying;
    host.store.save(&stopped).unwrap();
    assert!(status(&host, false).unwrap().interrupted);
    let going = status(&host, true).unwrap();
    assert!(going.running && !going.interrupted);

    assert!(!is_finished(&host).unwrap());
    move_all(&host).await;
    assert_eq!(status(&host, false).unwrap().state.phase, Phase::Cleaned);
    assert!(is_finished(&host).unwrap(), "nothing left to start");
}

#[tokio::test]
async fn a_cleaned_migration_with_an_unconfirmed_import_is_not_finished() {
    let host = FakeHost::with(2).await;
    host.unconfirmed.store(true, Ordering::SeqCst);
    move_all(&host).await;
    assert!(host.store.load().unwrap().rechecked);
    assert!(is_finished(&host).unwrap());

    let mut state = host.store.load().unwrap();
    state.rechecked = false;
    host.store.save(&state).unwrap();
    assert!(
        !is_finished(&host).unwrap(),
        "the import still has to be re-checked"
    );
}

#[tokio::test]
async fn retry_puts_a_lost_copy_back_in_line() {
    let host = FakeHost::with(4).await;
    move_all(&host).await;
    assert_eq!(status(&host, false).unwrap().state.phase, Phase::Cleaned);

    // A legacy item the move did not reach (written after cleanup).
    host.legacy.store(fact("late")).await.unwrap();
    let state = retry(&host, false).unwrap();
    assert!(!state.caught_up && !state.cleaning);
    run(&host, Trigger::Manual { takeover: false }, || async {
        false
    })
    .await
    .unwrap();
    assert_eq!(count(host.tree.as_ref()).await, 5);
    assert_eq!(count(host.legacy.as_ref()).await, 0);
}

#[tokio::test]
async fn retry_refuses_while_a_run_is_going() {
    let host = FakeHost::with(1).await;
    assert!(matches!(retry(&host, true), Err(Error::InvalidRequest(_))));
}

#[tokio::test]
async fn a_failed_run_is_recorded_as_paused_with_its_reason() {
    let host = FakeHost::with(1).await;
    record_failure(&host, &Error::Off("sign in".into()));
    let state = host.store.load().unwrap();
    assert_eq!(state.phase, Phase::Paused);
    assert_eq!(state.error.as_deref(), Some("memory is off: sign in"));
}

#[cfg(feature = "fs-claim")]
#[test]
fn filesystem_claim_normalizes_endpoints_and_preserves_first_winner() {
    use crate::layout_migration::{FsLegacyClaim, LegacyClaim};

    let tmp = tempfile::tempdir().unwrap();
    let first = FsLegacyClaim::new(tmp.path(), " HTTP://127.0.0.1:3141/ ", "user:first");
    let same_endpoint =
        FsLegacyClaim::new(tmp.path(), "http://127.0.0.1:3141", "user:second");
    let different_endpoint =
        FsLegacyClaim::new(tmp.path(), "http://127.0.0.1:3142", "user:second");
    assert_eq!(first.marker_path(), same_endpoint.marker_path());
    assert_ne!(first.marker_path(), different_endpoint.marker_path());
    assert!(!first.held_by_other().unwrap());
    assert!(first.take().unwrap());
    assert!(first.take().unwrap(), "the owner can safely take it again");
    assert!(same_endpoint.held_by_other().unwrap());
    assert!(!same_endpoint.take().unwrap());
    assert!(!first.held_by_other().unwrap());
    assert!(first.marker_path().is_file());
}

#[cfg(feature = "fs-claim")]
#[test]
fn filesystem_claim_race_has_one_winner_and_torn_markers_are_errors() {
    use crate::layout_migration::{FsLegacyClaim, LegacyClaim};
    use std::sync::Arc;

    let tmp = tempfile::tempdir().unwrap();
    let first = Arc::new(FsLegacyClaim::new(
        tmp.path(),
        "http://127.0.0.1:3141",
        "user:a",
    ));
    let second = Arc::new(FsLegacyClaim::new(
        tmp.path(),
        "http://127.0.0.1:3141",
        "user:b",
    ));
    let a = Arc::clone(&first);
    let b = Arc::clone(&second);
    let first_result = std::thread::spawn(move || a.take().unwrap());
    let second_result = std::thread::spawn(move || b.take().unwrap());
    let took_first = first_result.join().unwrap();
    let took_second = second_result.join().unwrap();
    assert_ne!(took_first, took_second, "hard-link creation has one winner");

    let winner = if took_first { first } else { second };
    let torn = FsLegacyClaim::new(tmp.path(), "http://127.0.0.1:3142", "user:torn");
    std::fs::create_dir_all(torn.marker_path().parent().unwrap()).unwrap();
    std::fs::write(torn.marker_path(), b"{ torn").unwrap();
    assert!(torn.held_by_other().is_err());
    assert!(torn.take().is_err());
    assert!(!winner.held_by_other().unwrap());
}
