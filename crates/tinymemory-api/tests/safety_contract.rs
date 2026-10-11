//! Public local safety contract regression.

use tinymemory_api::safety::{PiiCategory, RiskLevel, redact_identifiers, scan};

#[test]
fn local_safety_contract_reports_and_redacts_strong_identifiers() {
    let result = scan("patient John was diagnosed; SSN 123-45-6789");
    assert_eq!(result.level, RiskLevel::High);
    assert!(result.has_category(PiiCategory::NationalId));
    assert!(!redact_identifiers("SSN 123-45-6789").contains("123-45-6789"));
}
