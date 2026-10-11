# Local identification risk

The public `safety` contract exposes stable risk levels, categories and counts,
plus pure `scan` and structured `redact_identifiers` helpers. Results never
include matched substrings. Classification uses local regexes, keywords and
structural validators; it performs no I/O and links no engine, storage or
runtime. Consumers own whether a result blocks, warns or allows an operation.

Strong identifiers force high risk. Category co-occurrence adds the existing
score bonus; benign text and invalid cards/IPs remain unflagged. Redaction
replaces validated structured identifiers while preserving topical keywords.

This helper preserves the OpenHuman detector behavior and is independent of
`tinymemory-integrations::safety`, which owns storage-item scrubbing. It does
not install a write pipeline or enforce a privacy policy. Regex and log are
pure dependencies; diagnostics contain categories and scores only.

Risk vocabulary is available by default. Enable `local-safety` for regex-based
classification and redaction; the default engine contract stays regex-free.
