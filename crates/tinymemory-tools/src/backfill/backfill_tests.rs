use super::*;
use tinymemory_api::conformance::ReferenceEngine;

fn message(sender: &str, content: &str, at: &str) -> ThreadMessage {
    ThreadMessage::new(sender, content, at)
}

fn turn(user: &str, assistant: &str) -> PastTurn {
    PastTurn {
        user: user.into(),
        assistant: assistant.into(),
        at: Utc::now(),
    }
}

#[test]
fn messages_become_turns_like_the_chat_shows_them() {
    let messages = [
        message("agent", "Welcome!", "2026-09-01T09:59:00Z"),
        message("user", "Plan a trip", "2026-09-01T10:00:00Z"),
        message("agent", "Where to?", "2026-09-01T10:01:00Z"),
        message("agent", "Also, when?", "2026-09-01T10:02:00Z"),
        message("user", "   ", "2026-09-01T10:03:00Z"),
        message("User", "Bali in October", "not a time"),
    ];
    let turns = turns_of(&messages);
    assert_eq!(turns.len(), 3);
    assert_eq!(turns[0].user, "");
    assert_eq!(turns[0].assistant, "Welcome!");
    assert_eq!(turns[1].user, "Plan a trip");
    assert_eq!(turns[1].assistant, "Where to?\n\nAlso, when?");
    assert_eq!(turns[1].at.to_rfc3339(), "2026-09-01T10:02:00+00:00");
    assert_eq!(turns[2].user, "Bali in October");
    assert!(turns[2].assistant.is_empty());
}

#[test]
fn the_range_stops_where_live_logging_began() {
    assert_eq!(pending_range(10, None, 0), 0..10);
    assert_eq!(pending_range(10, Some(7), 0), 0..7, "turns 7.. are live's");
    assert_eq!(pending_range(10, Some(7), 4), 4..7);
    assert_eq!(pending_range(10, Some(7), 9), 7..7, "nothing left");
    assert_eq!(pending_range(2, Some(5), 0), 0..2);
}

#[test]
fn groups_hold_whole_turns() {
    let groups = turn_groups(3..3 + TURNS_PER_GROUP * 2 + 1);
    assert_eq!(groups.len(), 3);
    assert_eq!(groups[0].len(), TURNS_PER_GROUP);
    assert_eq!(groups[0][0], 3);
    assert_eq!(groups[2], vec![3 + TURNS_PER_GROUP * 2]);
    assert!(turn_groups(4..4).is_empty());
}

#[test]
fn a_turn_is_one_item_per_message_at_the_agents_node() {
    let layout = MemoryLayout::default();
    let past = turn("q", "a");
    let items = turn_items(&layout, MAIN_AGENT, "t", 3, &past);
    assert_eq!(items.len(), 2);
    let meta = items[1].meta();
    assert_eq!(meta.turns.map(|range| range.first), Some(7));
    assert_eq!(meta.namespace.to_string(), "agent:orchestrator");
    assert_eq!(meta.agent_id.as_deref(), Some(MAIN_AGENT));
    assert!(meta.tags.iter().any(|tag| tag == BACKFILL_TAG));
    let reply_only = PastTurn {
        user: String::new(),
        ..past
    };
    assert_eq!(
        turn_items(&layout, MAIN_AGENT, "t", 0, &reply_only).len(),
        1
    );
}

#[test]
fn pooled_conversations_land_at_the_pooled_node() {
    let layout = MemoryLayout::default()
        .with_pooled_conversations(&"ws:main".parse().unwrap())
        .unwrap();
    let items = turn_items(&layout, MAIN_AGENT, "t", 0, &turn("q", "a"));
    assert_eq!(items[0].meta().namespace.to_string(), "ws:main");
}

#[test]
fn a_group_flattens_its_turns_in_order_and_skips_unknown_indices() {
    let layout = MemoryLayout::default();
    let turns = [turn("q0", "a0"), turn("q1", "a1")];
    let items = group_items(&layout, MAIN_AGENT, "t", &turns, &[0, 1, 9]);
    let firsts: Vec<_> = items
        .iter()
        .map(|item| item.meta().turns.unwrap().first)
        .collect();
    assert_eq!(firsts, [0, 1, 2, 3]);
}

#[tokio::test]
async fn live_logging_is_found_by_the_first_untagged_turn() {
    let engine = ReferenceEngine::new();
    let layout = MemoryLayout::default();
    // Turn 0 was backfilled; turn 2 and 3 were logged live (no tag).
    for item in turn_items(&layout, MAIN_AGENT, "t", 0, &turn("q", "a")) {
        engine.store(item).await.unwrap();
    }
    assert_eq!(first_live_turn(&engine, "t").await, None);
    for item in turn_items(&layout, MAIN_AGENT, "t", 2, &turn("q", "a")) {
        let mut item = item;
        item.meta_mut().tags.clear();
        engine.store(item).await.unwrap();
    }
    assert_eq!(first_live_turn(&engine, "t").await, Some(2));
    assert_eq!(first_live_turn(&engine, "other").await, None);
}
