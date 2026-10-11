//! Data-only provenance policy and explicit markers for recalled external text.
//!
//! Hosts supply their own connector-prefix catalogue. This module does not
//! read memory, migrate metadata, or infer that a marker makes content safe.

/// Conservative legacy provenance rules with a host-supplied connector list.
///
/// Unknown namespaces are untrusted. Known connector prefixes override a
/// locally authored namespace; an absent namespace and unmatched key retain
/// the legacy locally authored interpretation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenancePolicy {
    connector_prefixes: Vec<String>,
}

impl ProvenancePolicy {
    /// Builds a policy from the host's connector key prefixes.
    pub fn new(prefixes: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self {
            connector_prefixes: prefixes
                .into_iter()
                .map(|prefix| prefix.as_ref().to_ascii_lowercase())
                .collect(),
        }
    }

    /// Whether an entry should be marked untrusted before entering a prompt.
    #[must_use]
    pub fn is_potentially_untrusted(&self, namespace: Option<&str>, key: &str) -> bool {
        if let Some(namespace) = namespace {
            let namespace = namespace.trim().to_ascii_lowercase();
            if !is_locally_authored_namespace(&namespace) {
                return true;
            }
        }
        let key = key.to_ascii_lowercase();
        self.connector_prefixes
            .iter()
            .any(|prefix| key.starts_with(prefix))
    }
}

fn is_locally_authored_namespace(namespace: &str) -> bool {
    matches!(
        namespace,
        "working" | "agent" | "local" | "core" | "global" | "default" | "user"
    ) || namespace.starts_with("working.")
        || namespace.starts_with("agent.")
        || namespace.starts_with("tree.")
}

/// Renders external context inside one explicit untrusted-source marker.
///
/// Ampersands and angle brackets in content are escaped so content cannot
/// terminate its own marker. The hint contains at most 64 ASCII alphanumeric,
/// hyphen, underscore, dot or colon characters and defaults to `external`.
#[must_use]
pub fn wrap_untrusted_for_agent(content: &str, source_hint: &str) -> String {
    let hint: String = source_hint
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
        .take(64)
        .collect();
    let hint = if hint.is_empty() { "external" } else { &hint };
    let content = content
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<untrusted-source source=\"{hint}\">\n{content}\n</untrusted-source>")
}
