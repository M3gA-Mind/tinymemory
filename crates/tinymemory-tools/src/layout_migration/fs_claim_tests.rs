//! Filesystem claim durability and first-writer behavior.

use super::*;

#[test]
fn claim_is_not_reported_owned_until_parent_directory_sync_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    let claim = FsLegacyClaim::new(temp.path(), "http://127.0.0.1:3141", "user:a");
    let expected_parent = claim.marker_path().parent().unwrap().to_path_buf();
    let calls = std::cell::Cell::new(0);

    let claimed = claim
        .take_with_directory_sync(|parent| {
            assert_eq!(parent, expected_parent);
            assert!(claim.marker_path().is_file());
            calls.set(calls.get() + 1);
            Ok(())
        })
        .unwrap();

    assert!(claimed);
    assert_eq!(calls.get(), 1);
}
