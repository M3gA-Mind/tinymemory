//! `store_many` on both wires: every item listed on return, ranked recall
//! probed for the last item only.

use super::*;
use crate::cortex::envelope::{decode_event, Envelope};
use crate::cortex::testing::{Shared, both, sample_items};
use tinymemory_api::{ListRequest, MemoryEngine, MemoryMeta, MetaFilter};

fn doc(text: &str) -> StoreItem {
    StoreItem::document(text, MemoryMeta::default())
}

fn conversation(channel: &str, thread: Option<&str>, namespace: &str) -> StoreItem {
    let mut items = sample_items();
    let mut item = items.remove(1);
    item.meta_mut().channel = Some(channel.to_string());
    item.meta_mut().thread_id = thread.map(str::to_string);
    item.meta_mut().namespace = namespace.parse().unwrap();
    item
}

fn seed_legacy_conversation(
    engine: &CortexEngine,
    state: &Shared,
    item: &StoreItem,
    event_count: usize,
) -> String {
    let id = item
        .legacy_conversation_fingerprint()
        .expect("channel-bearing conversations have a legacy id");
    for envelope in Envelope::for_item(item, &id).unwrap().into_iter().take(event_count) {
        let encoded = envelope.encode_checked().unwrap();
        let request = envelope.request(&encoded, &engine.layout);
        let (status, _) = state.log.lock().unwrap().append(&request);
        assert_eq!(status, 202);
    }
    id
}

fn event_write_count(state: &Shared) -> usize {
    state.seen.lock().unwrap().writes.len()
}

#[tokio::test]
async fn a_batch_is_listed_on_return_and_settles_only_its_last_item() {
    for (engine, state) in both().await {
        let wire = engine.wire();
        let items = vec![
            doc("first bulk note"),
            doc("second bulk note"),
            doc("third bulk note"),
        ];
        let receipts = engine.store_many(items.clone()).await.unwrap();
        let ids: Vec<String> = receipts.iter().map(|r| r.id.as_str().to_string()).collect();
        let fingerprints: Vec<String> = items.iter().map(StoreItem::fingerprint).collect();
        assert_eq!(ids, fingerprints, "{wire:?}: receipts in item order");

        let listed = engine
            .list(ListRequest::new(MetaFilter::default(), 10))
            .await
            .unwrap();
        assert_eq!(listed.items.len(), 3, "{wire:?}");

        let probed = |text: &str| {
            state
                .seen
                .lock()
                .unwrap()
                .recalls
                .iter()
                .filter(|body| body["query"].as_str().is_some_and(|q| q.contains(text)))
                .count()
        };
        assert_eq!(probed("first bulk note"), 0, "{wire:?}");
        assert_eq!(probed("second bulk note"), 0, "{wire:?}");
        assert!(
            probed("third bulk note") >= 1,
            "{wire:?}: the last item settles"
        );

        let again = engine.store_many(items).await.unwrap();
        assert!(again.iter().all(|r| r.replayed), "{wire:?}");
    }
}

#[tokio::test]
async fn an_empty_or_invalid_batch_is_refused() {
    for (engine, _state) in both().await {
        assert!(matches!(
            engine.store_many(Vec::new()).await,
            Err(crate::cortex::Error::InvalidRequest(_))
        ));
        assert!(engine.store_many(vec![doc("   ")]).await.is_err());
    }
}

#[tokio::test]
async fn a_repeat_inside_a_batch_and_mixed_kinds_are_handled() {
    use tinymemory_api::LearningKind;
    for (engine, _state) in both().await {
        let wire = engine.wire();
        let learning = StoreItem::learning(
            "prefers tea",
            LearningKind::Preference,
            0.9,
            MemoryMeta::default(),
        );
        let receipts = engine
            .store_many(vec![doc("bulk doc"), learning, doc("bulk doc")])
            .await
            .unwrap();
        let replayed: Vec<bool> = receipts.iter().map(|r| r.replayed).collect();
        assert_eq!(replayed, [false, false, true], "{wire:?}");
        assert_eq!(receipts[0].id, receipts[2].id);
        let listed = engine
            .list(ListRequest::new(MetaFilter::default(), 10))
            .await
            .unwrap();
        assert_eq!(
            listed.items.len(),
            2,
            "{wire:?}: the repeat was not written twice"
        );
    }
}

#[tokio::test]
async fn a_single_store_is_listed_and_settled_on_return_like_a_batch_of_one() {
    for (engine, state) in both().await {
        let wire = engine.wire();
        let receipt = engine.store(doc("single settled note")).await.unwrap();
        assert!(!receipt.replayed, "{wire:?}");
        assert_eq!(
            receipt.id.as_str(),
            doc("single settled note").fingerprint(),
            "{wire:?}"
        );
        let listed = engine
            .list(ListRequest::new(MetaFilter::default(), 10))
            .await
            .unwrap();
        assert_eq!(listed.items.len(), 1, "{wire:?}: listed on return");
        let probed = state
            .seen
            .lock()
            .unwrap()
            .recalls
            .iter()
            .filter(|body| {
                body["query"]
                    .as_str()
                    .is_some_and(|q| q.contains("single settled note"))
            })
            .count();
        assert!(
            probed >= 1,
            "{wire:?}: a single store waits for ranked recall"
        );
        assert!(
            engine
                .store(doc("single settled note"))
                .await
                .unwrap()
                .replayed,
            "{wire:?}"
        );
        assert!(
            matches!(
                engine.store(doc("   ")).await,
                Err(crate::cortex::Error::InvalidRequest(_))
            ),
            "{wire:?}"
        );
    }
}

#[tokio::test]
async fn a_channel_inclusive_legacy_id_without_a_thread_is_replayed() {
    for (engine, state) in both().await {
        let item = conversation("telegram", None, "root");
        let legacy_id = seed_legacy_conversation(&engine, &state, &item, usize::MAX);
        let writes = event_write_count(&state);

        let receipt = engine.store(item).await.unwrap();

        assert!(receipt.replayed, "{:?}", engine.wire());
        assert_eq!(receipt.id.as_str(), legacy_id, "{:?}", engine.wire());
        assert_eq!(event_write_count(&state), writes, "{:?}", engine.wire());
    }
}

#[tokio::test]
async fn a_changed_channel_replays_only_inside_its_namespace() {
    for (engine, state) in both().await {
        let legacy = conversation("telegram", Some("thread-1"), "team:alpha");
        let legacy_id = seed_legacy_conversation(&engine, &state, &legacy, usize::MAX);
        let writes = event_write_count(&state);

        let same_scope = conversation("slack", Some("thread-1"), "team:alpha");
        let replay = engine.store(same_scope).await.unwrap();
        assert!(replay.replayed, "{:?}", engine.wire());
        assert_eq!(replay.id.as_str(), legacy_id, "{:?}", engine.wire());
        assert_eq!(event_write_count(&state), writes, "{:?}", engine.wire());

        let other_scope = conversation("slack", Some("thread-1"), "team:beta");
        let new_id = other_scope.fingerprint();
        let isolated = engine.store(other_scope).await.unwrap();
        assert!(!isolated.replayed, "{:?}", engine.wire());
        assert_eq!(isolated.id.as_str(), new_id, "{:?}", engine.wire());
    }
}

#[tokio::test]
async fn a_partial_legacy_conversation_continues_under_its_existing_id() {
    for (engine, state) in both().await {
        let item = conversation("telegram", Some("thread-2"), "team:alpha");
        let legacy_id = seed_legacy_conversation(&engine, &state, &item, 2);

        let receipt = engine.store(item.clone()).await.unwrap();

        assert!(!receipt.replayed, "{:?}", engine.wire());
        assert_eq!(receipt.id.as_str(), legacy_id, "{:?}", engine.wire());
        let mut stored: Vec<_> = state
            .log
            .lock()
            .unwrap()
            .events
            .iter()
            .filter_map(decode_event)
            .filter(|event| event.envelope.id == legacy_id)
            .map(|event| event.envelope.turn.unwrap().index)
            .collect();
        stored.sort_unstable();
        assert_eq!(stored, vec![0, 1, 2], "{:?}", engine.wire());
    }
}

fn turn(text: &str) -> StoreItem {
    StoreItem::Conversation {
        turns: vec![tinymemory_api::Turn::new(tinymemory_api::Role::User, text)],
        meta: MemoryMeta {
            thread_id: Some("t-hot".into()),
            ..MemoryMeta::default()
        },
    }
}

#[tokio::test]
async fn a_logged_turn_checks_legacy_identity_and_a_retry_is_a_replay() {
    use tinymemory_api::WriteOptions;
    let (endpoint, state) = crate::cortex::testing::direct_double().await;
    let engine = crate::cortex::testing::direct_engine(&endpoint);
    let first = engine
        .store_with(turn("Ship it on Friday."), WriteOptions::accepted())
        .await
        .unwrap();
    assert!(!first.replayed);
    assert_eq!(
        state.count("GET /v1/events"),
        2,
        "current id lookup plus the changed-channel thread fallback"
    );
    let retry = engine
        .store_with(turn("Ship it on Friday."), WriteOptions::accepted())
        .await
        .unwrap();
    assert!(retry.replayed, "CortexDB answered the retry as a replay");
    assert_eq!(retry.id, first.id);
    assert_eq!(state.event_count(), 1, "written once");
    assert_eq!(state.count("GET /v1/events"), 3);
}

#[tokio::test]
async fn an_unidentified_logged_turn_still_skips_the_lookup() {
    use tinymemory_api::WriteOptions;
    let (endpoint, state) = crate::cortex::testing::direct_double().await;
    let engine = crate::cortex::testing::direct_engine(&endpoint);
    let item = StoreItem::Conversation {
        turns: vec![tinymemory_api::Turn::new(
            tinymemory_api::Role::User,
            "No thread or channel identity.",
        )],
        meta: MemoryMeta::default(),
    };
    let first = engine
        .store_with(item.clone(), WriteOptions::accepted())
        .await
        .unwrap();
    let retry = engine.store_with(item, WriteOptions::accepted()).await.unwrap();
    assert!(retry.replayed);
    assert_eq!(retry.id, first.id);
    assert_eq!(state.event_count(), 1);
    assert_eq!(state.count("GET /v1/events"), 0);
}

#[tokio::test]
async fn every_other_store_still_looks_its_items_up_first() {
    use tinymemory_api::WriteOptions;
    for (engine, state) in both().await {
        let listing = match engine.wire() {
            crate::cortex::CortexWire::Direct | crate::cortex::CortexWire::Proxied => {
                "GET /v1/events"
            }
            crate::cortex::CortexWire::TinyHumans => "GET /memory/events",
        };
        let cases: Vec<(StoreItem, WriteOptions)> = vec![
            (doc("A synced file."), WriteOptions::accepted()),
            (turn("Waited for."), WriteOptions::visible()),
        ];
        for (item, options) in cases {
            let before = state.count(listing);
            engine.store_with(item.clone(), options).await.unwrap();
            assert!(state.count(listing) > before, "{:?} looked up", item.kind());
        }
        if engine.wire() == crate::cortex::CortexWire::TinyHumans {
            let before = state.count(listing);
            engine
                .store_with(turn("Hosted turn."), WriteOptions::accepted())
                .await
                .unwrap();
            assert!(state.count(listing) > before, "hosted always looks up");
        }
    }
}
