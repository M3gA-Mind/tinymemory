//! Local PII / identification-risk detector (pure public contract helper).
//!
//! Detects when a prompt or document may contain patient / legal / financial /
//! family or other identifiable information, and reports a risk **level**, a
//! numeric **score**, and the matched **categories** — so upstream gates can
//! decide whether content is safe to send off-device.
//!
//! ## Hard guarantee: detection is fully local
//! Classification is pure pattern + keyword matching over the input string.
//! There is **no** network call, no async, no external model — importing this
//! module pulls in nothing beyond `regex`. Sending content to a remote service
//! to classify it would defeat the entire purpose of the privacy epic, so the
//! helper performs no I/O and requires no runtime or engine.
//!
//! ## Recall over precision
//! A missed flag means sensitive data leaves the machine unnoticed, so the
//! detector is tuned to over-flag rather than under-flag. See `scan` (feature `local-safety`)
//! for the scoring model.
//!
//! ```
//! # #[cfg(feature = "local-safety")]
//! # {
//! use tinymemory_api::safety::{scan, PiiCategory, RiskLevel};
//!
//! let result = scan("patient John was diagnosed; SSN 123-45-6789");
//! assert_eq!(result.level, RiskLevel::High);
//! assert!(result.has_category(PiiCategory::NationalId));
//! # }
//! ```

#[cfg(feature = "local-safety")]
mod detector;
#[cfg(feature = "local-safety")]
mod rules;
mod types;

#[cfg(feature = "local-safety")]
pub use detector::{REDACTED_IDENTIFIER, redact_identifiers, scan};
pub use types::{CategoryHit, PiiCategory, PiiScanResult, RiskLevel};

#[cfg(all(test, feature = "local-safety"))]
#[path = "mod_tests.rs"]
mod tests;
