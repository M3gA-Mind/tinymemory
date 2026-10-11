# Recalled context provenance

`ProvenancePolicy` classifies legacy namespace/key pairs conservatively. Hosts supply connector prefixes; the contract owns the locally authored namespace families. Unknown namespaces are untrusted, and a connector key prefix overrides a locally authored namespace.

`wrap_untrusted_for_agent` renders external context inside one escaped marker. The bounded hint is an ASCII identifier; content cannot inject a closing tag. The marker conveys provenance, not a guarantee that a model ignores instructions inside it.

These are data-only contract operations. They do not depend on an engine implementation or change stored memory metadata. The accepted behavior is in [the spec](../../../../docs/specs/provenance-markers.md).
