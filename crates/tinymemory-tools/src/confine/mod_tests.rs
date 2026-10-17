//! Tests for namespace confinement policies.

use super::*;
use tinymemory_api::{LearningKind, SourceKind, SourceRef};

fn ns(value: &str) -> Namespace {
    value.parse().unwrap()
}

fn confinement(policy: Policy) -> Confinement {
    Confinement {
        allowed: Reach::subtree(ns("team:acme")),
        policy,
    }
}

fn learning_at(namespace: &str) -> StoreItem {
    StoreItem::learning(
        "fact",
        LearningKind::Fact,
        0.8,
        MemoryMeta {
            namespace: ns(namespace),
            source: SourceRef {
                kind: SourceKind::Agent,
                id: None,
            },
            ..MemoryMeta::default()
        },
    )
}

#[test]
fn refuse_uses_the_allowed_reach_and_rejects_wider_requests() {
    let scope = confinement(Policy::Refuse);
    assert_eq!(scope.reach(None).unwrap(), scope.allowed);

    let narrower = Reach::exact(ns("team:acme/agent:writer"));
    assert_eq!(scope.reach(Some(narrower.clone())).unwrap(), narrower);

    for wider in [
        Reach::subtree(Namespace::ROOT),
        Reach::subtree(ns("team:other")),
        Reach::of(ns("team:acme/agent:writer")),
    ] {
        assert!(matches!(
            scope.reach(Some(wider)),
            Err(Error::InvalidRequest(_))
        ));
    }
}

#[test]
fn clamp_keeps_inside_reads_but_removes_ancestors() {
    let scope = confinement(Policy::Clamp);
    assert_eq!(scope.reach(None).unwrap(), scope.allowed);
    assert_eq!(
        scope.reach(Some(Reach::of(Namespace::ROOT))).unwrap(),
        scope.allowed,
        "a reach from the global root would include other tenants"
    );
    assert_eq!(
        scope.reach(Some(Reach::subtree(ns("team:other")))).unwrap(),
        scope.allowed,
        "a sibling tenant is clamped to the allowed root"
    );

    let inside = scope
        .reach(Some(Reach::of(ns("team:acme/agent:writer"))))
        .unwrap();
    assert_eq!(inside.at, ns("team:acme/agent:writer"));
    assert!(!inside.inherit, "ancestors can escape the allowed root");

    let exact_root = scope
        .reach(Some(Reach::exact(ns("team:acme"))))
        .unwrap();
    assert_eq!(exact_root, Reach::exact(ns("team:acme")));

    let subtree_root = scope
        .reach(Some(Reach::subtree(ns("team:acme"))))
        .unwrap();
    assert_eq!(subtree_root, scope.allowed);

    let inherited_root = scope
        .reach(Some(Reach::of(ns("team:acme"))))
        .unwrap();
    assert_eq!(inherited_root, Reach::exact(ns("team:acme")));
}

#[test]
fn filters_keep_other_clauses_while_confining_the_reach() {
    let scope = confinement(Policy::Clamp);
    let filter = MetaFilter {
        reach: Some(Reach::subtree(Namespace::ROOT)),
        workspace: Some("repo".into()),
        ..MetaFilter::default()
    };
    let clamped = scope.filter(Some(filter)).unwrap();
    assert_eq!(clamped.reach, Some(scope.allowed));
    assert_eq!(clamped.workspace.as_deref(), Some("repo"));

    let scope = confinement(Policy::Refuse);
    assert!(matches!(
        scope.filter(Some(MetaFilter {
            reach: Some(Reach::subtree(Namespace::ROOT)),
            ..MetaFilter::default()
        })),
        Err(Error::InvalidRequest(_))
    ));
}

#[test]
fn clamp_moves_out_of_scope_items_and_preserves_inside_items() {
    let scope = confinement(Policy::Clamp);
    let mut outside = learning_at("team:other");
    scope.item(&mut outside).unwrap();
    assert_eq!(outside.meta().namespace, scope.allowed.at);

    let mut inside = learning_at("team:acme/agent:writer");
    scope.item(&mut inside).unwrap();
    assert_eq!(inside.meta().namespace, ns("team:acme/agent:writer"));
}

#[test]
fn learn_metadata_obeys_the_selected_policy() {
    let clamp = confinement(Policy::Clamp);
    let clamped = clamp
        .learn_meta(Some(MemoryMeta {
            namespace: ns("team:other"),
            ..MemoryMeta::default()
        }))
        .unwrap();
    assert_eq!(clamped.namespace, clamp.allowed.at);

    let refuse = confinement(Policy::Refuse);
    assert!(matches!(
        refuse.learn_meta(Some(MemoryMeta {
            namespace: ns("team:other"),
            ..MemoryMeta::default()
        })),
        Err(Error::InvalidRequest(_))
    ));
}
