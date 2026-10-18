use super::*;

async fn seed_green_tea(engine: &ReferenceEngine, scope: &ToolScope) {
    engine
        .store(StoreItem::learning(
            "prefers green tea",
            LearningKind::Preference,
            0.9,
            MemoryMeta {
                namespace: scope.place.clone(),
                ..MemoryMeta::default()
            },
        ))
        .await
        .expect("seed memory");
}

#[tokio::test]
async fn single_call_accepts_padded_action_for_recall() {
    let engine = Arc::new(ReferenceEngine::new());
    let scope = scope();
    seed_green_tea(&engine, &scope).await;
    let tool = SingleTool::new("memory", MemoryTools::with_scope(engine, scope));

    let result = tool
        .call(
            json!({ "action": " recall ", "question": "tea" }),
            CallContext::default(),
        )
        .await
        .expect("padded recall action is accepted");

    assert_eq!(result["items"], 1);
}

#[tokio::test]
async fn single_call_accepts_padded_action_for_fetch() {
    let engine = Arc::new(ReferenceEngine::new());
    let scope = scope();
    seed_green_tea(&engine, &scope).await;
    let tool = SingleTool::new("memory", MemoryTools::with_scope(engine, scope));

    let result = tool
        .call(
            json!({ "action": " fetch ", "query": "tea", "mode": "keyword" }),
            CallContext::default(),
        )
        .await
        .expect("padded fetch action is accepted");

    assert_eq!(result["items"], 1);
}

#[tokio::test]
async fn single_call_accepts_padded_action_for_learn() {
    let engine = Arc::new(ReferenceEngine::new());
    let scope = scope();
    let queue = queue(8);
    let tool = queued_tool(
        engine,
        scope,
        queue.clone(),
        Arc::new(WriteGate::new("padded-learn")),
    );

    let result = tool
        .call(
            json!({ "action": " learn ", "text": "likes jasmine tea" }),
            CallContext::default(),
        )
        .await
        .expect("padded learn action is accepted");

    assert_eq!(result["status"], "queued; not yet saved to memory");
    assert_eq!(queue.entries().expect("queued learn").len(), 1);
}

#[tokio::test]
async fn single_call_accepts_padded_action_for_forget() {
    let engine = Arc::new(ReferenceEngine::new());
    let scope = scope();
    let queue = queue(8);
    let tool = queued_tool(
        engine,
        scope,
        queue.clone(),
        Arc::new(WriteGate::new("padded-forget")),
    );

    let result = tool
        .call(
            json!({ "action": " forget ", "ids": ["missing-id"] }),
            CallContext::default(),
        )
        .await
        .expect("padded forget action is accepted");

    assert_eq!(result["status"], "queued; not yet removed from memory");
    let entries = queue.entries().expect("queued forget");
    assert!(matches!(
        &entries[0].operation,
        WriteOperation::Forget { .. }
    ));
}
