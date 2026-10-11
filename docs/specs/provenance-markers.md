# Provenance markers in recalled context

Recalled text can contain instructions authored by an external source. TinyMemory owns the data-only contract for classifying legacy namespace/key provenance and rendering explicit untrusted-source markers. No engine schema or stored-item migration is required.

Hosts supply connector prefixes to `ProvenancePolicy`; TinyMemory contains no connector catalogue. Unknown namespaces are untrusted, while the existing locally authored namespace families remain trusted. Matching is case-insensitive and namespaces are trimmed. A known connector prefix overrides a locally authored namespace.

`wrap_untrusted_for_agent` escapes ampersands and angle brackets in content, restricts the source hint to a bounded ASCII identifier, and emits one `<untrusted-source>` block. A payload cannot terminate its own marker. This renderer does not claim to eliminate prompt injection; it preserves the trust label at the text boundary.
