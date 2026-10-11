# Provenance marker implementation

Implements [the provenance marker contract](../specs/provenance-markers.md).

1. Add public API integration cases in `crates/tinymemory-api/tests/provenance.rs` for host-supplied prefixes, unknown namespaces, content escaping and bounded hints. Run the target before the API exists and confirm the missing-export failure.
2. Add `crates/tinymemory-api/src/provenance/mod.rs` with `ProvenancePolicy` and `wrap_untrusted_for_agent`, preserving the OpenHuman namespace and escaping behavior. Export the public types/functions from `src/lib.rs`. Run the integration target and the API tests.
3. Describe the contract in the module README and crate documentation; run formatting, Clippy, all-target builds, all-feature tests and rustdoc for the complete workspace before opening the upstream PR.
