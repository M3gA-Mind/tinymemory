use tinymemory_api::Error;

use super::*;

#[test]
fn a_direct_account_wide_error_uses_the_same_refusal_notice() {
    let pack = TurnPack::from_outcome(
        Err(Error::Unavailable("private transport detail".into())),
        true,
        "test-engine".into(),
    )
    .expect("account-wide direct errors should create a refusal notice");
    assert_eq!(pack.refusal, Some("UNAVAILABLE"));
    assert_eq!(pack.engine, "test-engine");
    assert!(pack.markdown.contains("could not be reached"));
    assert!(!pack.markdown.contains("private transport detail"));
}

#[test]
fn a_direct_non_account_wide_error_degrades_to_no_pack() {
    assert!(
        TurnPack::from_outcome(
            Err(Error::Engine("private engine detail".into())),
            true,
            "test-engine".into(),
        )
        .is_none()
    );
}
