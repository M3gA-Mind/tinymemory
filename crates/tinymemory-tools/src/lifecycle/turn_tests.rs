//! Public contract tests for the runtime-neutral turn lifecycle hooks.

use serde_json::json;
use tinymemory_api::{TimeHint, ToolCallRef, chrono::NaiveDate};

use super::turn::{
    InPromptFrom, LoggedToolCall, MAX_TOOL_LINE_CHARS, logged_reply, parse_range,
    user_turn_index,
};
use super::{PreTurn, RecallPolicy};

#[test]
fn resumed_defaults_false_for_existing_serialized_turns() {
    let turn: PreTurn = serde_json::from_value(json!({
        "thread_id": "thread",
        "turn_index": 4,
        "user_text": "hello"
    }))
    .unwrap();
    assert!(!turn.resumed);

    let resumed: PreTurn = serde_json::from_value(json!({
        "thread_id": "thread",
        "turn_index": 4,
        "user_text": "hello",
        "resumed": true
    }))
    .unwrap();
    assert!(resumed.resumed);
}

#[test]
fn log_only_policy_keeps_logging_possible_but_disables_every_recall_section() {
    let policy = RecallPolicy::default().log_only();
    assert_eq!(policy.learnings_limit, 0);
    assert_eq!(policy.brain_limit, 0);
    assert_eq!(policy.history_limit, 0);
    assert_eq!(policy.team_limit, 0);
    assert_eq!(policy.build_beliefs_every, None);
}

#[test]
fn turn_indices_saturate_without_wrapping() {
    assert_eq!(user_turn_index(0), 0);
    assert_eq!(user_turn_index(3), 6);
    assert_eq!(user_turn_index(usize::MAX), u32::MAX - 1);
}

#[test]
fn logged_reply_keeps_one_bounded_line_per_tool_result() {
    let calls = vec![
        LoggedToolCall {
            call: ToolCallRef {
                name: "web_search".into(),
                id: Some("call-1".into()),
            },
            result: Some("  Rust 1.90\n released   today ".into()),
        },
        LoggedToolCall {
            call: ToolCallRef {
                name: "noop".into(),
                id: None,
            },
            result: None,
        },
        LoggedToolCall {
            call: ToolCallRef {
                name: "huge".into(),
                id: None,
            },
            result: Some("🙂".repeat(5_000)),
        },
        LoggedToolCall {
            call: ToolCallRef {
                name: "empty".into(),
                id: None,
            },
            result: Some(" \n\t ".into()),
        },
    ];

    let text = logged_reply(" Done. ", &calls);
    assert!(text.starts_with("Done.\n\nTools:\n- web_search → Rust 1.90 released today"));
    assert!(!text.contains("noop"));
    let huge = text.lines().last().unwrap();
    assert!(huge.chars().count() <= MAX_TOOL_LINE_CHARS + "- huge → ".chars().count());
    assert!(huge.ends_with(&"🙂".repeat(MAX_TOOL_LINE_CHARS)));
    assert!(!text.contains("empty"));
    assert_eq!(logged_reply("plain", &[]), "plain");
    assert_eq!(
        logged_reply(
            "",
            &[LoggedToolCall {
                call: ToolCallRef {
                    name: "only_tool".into(),
                    id: None,
                },
                result: Some("result".into()),
            }]
        ),
        "\n\nTools:\n- only_tool → result"
    );
}

#[test]
fn the_live_window_fingerprints_folds_and_respects_its_ceiling() {
    let window = InPromptFrom::new(2, 20);
    assert_eq!(window.advance_past(["a", "b"]), 6);
    assert_eq!(window.advance_past(["c"]), 8);
    assert_eq!(window.advance_past(["c"]), 8);
    // A transcript folded again from the turn's original head counts from
    // that head, not on top of the already advanced index.
    assert_eq!(window.advance_past(["a", "b", "c"]), 8);
    assert_eq!(InPromptFrom::new(30, 20).get(), 20);

    let capped = InPromptFrom::new(16, 20);
    assert_eq!(capped.advance_past(["a", "b", "c"]), 20);
}

#[test]
fn date_hint_range_parser_handles_prose_fences_single_days_and_bad_ranges() {
    let day = |day| NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
    let parsed = parse_range(
        "```json\n{\"from\":\"2026-10-06\",\"to\":\"2026-10-08\"}\n```",
        Some("Asia/Kolkata"),
    )
    .unwrap();
    assert_eq!(
        parsed,
        TimeHint::new(day(6), day(8), Some("Asia/Kolkata".into())).unwrap()
    );
    let single = parse_range("{\"from\":\"2026-10-03\"}", Some("UTC")).unwrap();
    assert_eq!((single.from, single.to), (day(3), day(3)));
    for answer in [
        "{\"from\":null,\"to\":null}",
        "no date here",
        "{\"from\":\"last Saturday\"}",
        "{\"from\":\"2026-10-08\",\"to\":\"2026-10-06\"}",
        "} {",
    ] {
        assert_eq!(parse_range(answer, Some("UTC")), None, "{answer}");
    }
}
