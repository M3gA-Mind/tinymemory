use std::sync::atomic::Ordering;

use tinymemory_api::MemoryEngine;

use super::super::job::{Trigger, run};
use super::super::state::MigrationStateStore;
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
    assert!(!is_finished(&host).unwrap(), "the import still has to be re-checked");
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
    run(&host, Trigger::Manual { takeover: false }, || async { false })
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
