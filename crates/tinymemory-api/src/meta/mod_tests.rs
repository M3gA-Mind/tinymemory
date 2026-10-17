//! Metadata serialisation and defaults.

use super::*;

#[test]
fn a_default_meta_names_the_agent_as_its_source() {
    let meta = MemoryMeta::default();
    assert_eq!(meta.source.kind, SourceKind::Agent);
    assert_eq!(meta.source.id, None);
}

#[test]
fn unset_fields_are_omitted_and_round_trip() {
    let mut meta = MemoryMeta::from_source(SourceKind::Github, Some("src_1".into()));
    meta.repo = Some("owner/name".into());
    meta.tool_call = Some(ToolCallRef {
        name: "grep".into(),
        id: None,
    });
    meta.channel = Some("telegram".into());
    let json = serde_json::to_value(&meta).expect("serialise");
    assert_eq!(
        json,
        serde_json::json!({
            "repo": "owner/name",
            "channel": "telegram",
            "tool_call": { "name": "grep" },
            "source": { "kind": "github", "id": "src_1" }
        })
    );
    let back: MemoryMeta = serde_json::from_value(json).expect("deserialise");
    assert_eq!(back, meta);
}

#[test]
fn host_metadata_overlays_set_fields_and_preserves_unset_fields() {
    let caller = MemoryMeta {
        repo: Some("caller/repo".into()),
        channel: Some("caller-channel".into()),
        tags: vec!["caller-tag".into()],
        ..MemoryMeta::default()
    };
    let host = MemoryMeta {
        repo: Some("host/repo".into()),
        observed_actor: Some(ObservedActor {
            id: "user:+15551234567".into(),
            name: Some("Mum".into()),
        }),
        ..MemoryMeta::default()
    };

    let overlaid = caller.overlay(host);
    assert_eq!(overlaid.repo.as_deref(), Some("host/repo"));
    assert_eq!(overlaid.channel.as_deref(), Some("caller-channel"));
    assert_eq!(overlaid.tags, ["caller-tag"]);
    assert_eq!(
        overlaid.observed_actor,
        Some(ObservedActor {
            id: "user:+15551234567".into(),
            name: Some("Mum".into()),
        })
    );
}

#[test]
fn host_values_replace_namespace_source_and_tags_when_present() {
    let caller = MemoryMeta {
        namespace: "agent:caller".parse().unwrap(),
        source: SourceRef {
            kind: SourceKind::File,
            id: Some("caller".into()),
        },
        tags: vec!["caller".into()],
        ..MemoryMeta::default()
    };
    let host = MemoryMeta {
        namespace: "agent:host".parse().unwrap(),
        source: SourceRef {
            kind: SourceKind::Conversation,
            id: Some("host".into()),
        },
        tags: vec!["host".into()],
        ..MemoryMeta::default()
    };

    let overlaid = caller.overlay(host);
    assert_eq!(overlaid.namespace, "agent:host".parse().unwrap());
    assert_eq!(overlaid.source.kind, SourceKind::Conversation);
    assert_eq!(overlaid.source.id.as_deref(), Some("host"));
    assert_eq!(overlaid.tags, ["host"]);
}

#[test]
fn channel_sender_actor_ids_keep_host_normalization_rules() {
    assert_eq!(
        ObservedActor::for_channel_sender("whatsapp", "+15551234567", Some(" Mum ")),
        Some(ObservedActor {
            id: "user:+15551234567".into(),
            name: Some("Mum".into()),
        })
    );
    assert_eq!(
        ObservedActor::for_channel_sender("signal", "+1 (555) 123-4567", None)
            .map(|actor| actor.id)
            .as_deref(),
        Some("user:+15551234567")
    );
    assert_eq!(
        ObservedActor::for_channel_sender("SMS", "5551234567", None)
            .map(|actor| actor.id)
            .as_deref(),
        Some("user:5551234567")
    );
    assert_eq!(
        ObservedActor::for_channel_sender("imessage", "Priya@Acme.com", None)
            .map(|actor| actor.id)
            .as_deref(),
        Some("user:priya@acme.com")
    );
    assert_eq!(
        ObservedActor::for_channel_sender("email", "priya@acme.com", None)
            .map(|actor| actor.id)
            .as_deref(),
        Some("user:priya@acme.com")
    );
    assert_eq!(
        ObservedActor::for_channel_sender("telegram", "123456789", None)
            .map(|actor| actor.id)
            .as_deref(),
        Some("telegram:123456789")
    );
    assert_eq!(
        ObservedActor::for_channel_sender("whatsapp", "98765432101234@lid", None)
            .map(|actor| actor.id)
            .as_deref(),
        Some("whatsapp:98765432101234@lid")
    );
    assert_eq!(ObservedActor::for_channel_sender("slack", "two words", None), None);
    assert_eq!(ObservedActor::for_channel_sender(" ", "+15551234567", None), None);
    assert_eq!(ObservedActor::for_channel_sender("whatsapp", "  ", Some("Mum")), None);
}

#[test]
fn an_empty_object_deserialises_to_the_default() {
    let meta: MemoryMeta = serde_json::from_str("{}").expect("deserialise");
    assert_eq!(meta, MemoryMeta::default());
}

#[test]
fn source_kind_wire_strings_match_serde() {
    for kind in SourceKind::ALL {
        let json = serde_json::to_value(kind).expect("serialise");
        assert_eq!(json, serde_json::json!(kind.as_str()));
    }
}

#[test]
fn the_retired_composio_source_kind_decodes_as_import() {
    let kind: SourceKind =
        serde_json::from_value(serde_json::json!("composio")).expect("legacy kind decodes");
    assert_eq!(kind, SourceKind::Import);
    assert_eq!(
        serde_json::to_value(SourceKind::Import).expect("serialise"),
        serde_json::json!("import")
    );
}
