//! Tests for the `[CODE]` prefix helpers.

use super::*;

#[test]
fn a_hosted_code_is_read_back_from_the_prefix() {
    let error = Error::Unauthorized("[UNAUTHORIZED] memory API memory/events — expired".into());
    assert_eq!(error_code(&error), Some("UNAUTHORIZED"));
}

#[test]
fn an_unprefixed_message_has_no_code() {
    assert_eq!(error_code(&Error::Engine("HTTP 500".into())), None);
    assert_eq!(error_code(&Error::Engine("[lowercase] nope".into())), None);
    assert_eq!(error_code(&Error::Engine("[] empty".into())), None);
}

#[test]
fn insufficient_credits_needs_both_the_variant_and_the_code() {
    let credits = Error::Engine(format!("[{INSUFFICIENT_CREDITS_CODE}] top up"));
    assert!(is_insufficient_credits(&credits));
    let wrong_variant = Error::Unavailable(format!("[{INSUFFICIENT_CREDITS_CODE}] top up"));
    assert!(!is_insufficient_credits(&wrong_variant));
    assert!(!is_insufficient_credits(&Error::Engine(
        "[RATE_LIMITED] slow".into()
    )));
}

#[test]
fn plan_required_needs_the_engine_variant_and_a_plan_code() {
    assert!(is_plan_required(&Error::Engine(
        "[MEMORY_PLAN_REQUIRED] not in the plan".into()
    )));
    assert!(!is_plan_required(&Error::Unauthorized(
        "[MEMORY_PLAN_REQUIRED] not in the plan".into()
    )));
    assert!(!is_plan_required(&Error::Engine(
        "[USER_INSUFFICIENT_CREDITS] top up".into()
    )));
    assert!(!is_plan_required(&Error::Engine(
        "MEMORY_PLAN_REQUIRED without the prefix".into()
    )));
}
