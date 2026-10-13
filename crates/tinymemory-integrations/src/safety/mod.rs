//! Secret and PII scrubbing for a [`tinymemory_api::StoreItem`].
//!
//! The scrubbers themselves (`sanitize_text_with`, `sanitize_json_with`,
//! [`Policy`] and the PII module) live in the standalone `tinyredact` crate,
//! which has no memory dependency; this module re-exports them so a memory host
//! has one import, and adds [`scrub_item`], the only part that needs the
//! `StoreItem` type. [`scrub_item`] applies the policy to every text an item
//! carries and is what a host runs on each item before `MemoryEngine::store`.

mod item;

pub use item::{scrub_item, scrub_item_with};
pub use tinyredact::{
    BareCardGate, Policy, SanitizationReport, Sanitized, has_likely_email, has_likely_pii,
    has_likely_secret, pii, redact_credential_markers, sanitize_json, sanitize_json_with,
    sanitize_text, sanitize_text_with,
};
