use super::*;

const CREATE_INDEXER: &str = r#"mutation($input: CreateIndexerConfigInput!) {
    createIndexerConfig(input: $input) { id maxQueriesPerMinute }
}"#;

const UPDATE_INDEXER: &str = r#"mutation($input: UpdateIndexerConfigInput!) {
    updateIndexerConfig(input: $input) { id name maxQueriesPerMinute }
}"#;

const READ_INDEXERS: &str = r#"query { indexers { id name maxQueriesPerMinute } }"#;

fn create_input(max_queries_per_minute: Value) -> Value {
    json!({
        "input": {
            "name": "Synthetic Budget Indexer",
            "providerType": "newznab",
            "isEnabled": false,
            "maxQueriesPerMinute": max_queries_per_minute,
            "config": [
                { "key": "base_url", "stringValue": "https://indexer.example.test" },
                { "key": "api_key", "stringValue": "synthetic-api-key" }
            ]
        }
    })
}

async fn stored_budget(ctx: &TestContext, id: &str) -> Value {
    let body = gql(ctx, READ_INDEXERS, json!({})).await;
    assert_no_errors(&body);
    body["data"]["indexers"]
        .as_array()
        .expect("indexers array")
        .iter()
        .find(|indexer| indexer["id"] == id)
        .unwrap_or_else(|| panic!("indexer {id} should be listed: {body}"))["maxQueriesPerMinute"]
        .clone()
}

#[tokio::test]
async fn graphql_indexer_query_budget_is_set_preserved_and_cleared() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let created = gql(&ctx, CREATE_INDEXER, create_input(json!(20))).await;
    assert_no_errors(&created);
    assert_eq!(
        created["data"]["createIndexerConfig"]["maxQueriesPerMinute"],
        20
    );
    let id = created["data"]["createIndexerConfig"]["id"]
        .as_str()
        .expect("created indexer id")
        .to_string();
    assert_eq!(stored_budget(&ctx, &id).await, 20);

    let replaced = gql(
        &ctx,
        UPDATE_INDEXER,
        json!({ "input": { "id": id, "maxQueriesPerMinute": 45 } }),
    )
    .await;
    assert_no_errors(&replaced);
    assert_eq!(
        replaced["data"]["updateIndexerConfig"]["maxQueriesPerMinute"],
        45
    );
    assert_eq!(stored_budget(&ctx, &id).await, 45);

    let renamed = gql(
        &ctx,
        UPDATE_INDEXER,
        json!({ "input": { "id": id, "name": "Renamed Budget Indexer" } }),
    )
    .await;
    assert_no_errors(&renamed);
    assert_eq!(
        renamed["data"]["updateIndexerConfig"]["maxQueriesPerMinute"], 45,
        "omitting the budget preserves it"
    );

    let cleared = gql(
        &ctx,
        UPDATE_INDEXER,
        json!({ "input": { "id": id, "maxQueriesPerMinute": null } }),
    )
    .await;
    assert_no_errors(&cleared);
    assert_eq!(
        cleared["data"]["updateIndexerConfig"]["maxQueriesPerMinute"],
        Value::Null
    );
    assert_eq!(stored_budget(&ctx, &id).await, Value::Null);
}

#[tokio::test]
async fn graphql_indexer_query_budget_rejects_zero() {
    let ctx = TestContext::new().await;
    seed_typed_settings_definitions(&ctx).await;

    let refused_create = gql(&ctx, CREATE_INDEXER, create_input(json!(0))).await;
    let (message, _) = first_graphql_error_message_and_code(&refused_create);
    assert!(
        message.contains("max queries per minute"),
        "unexpected error: {refused_create}"
    );
    let listed = gql(&ctx, READ_INDEXERS, json!({})).await;
    assert_no_errors(&listed);
    assert!(
        !listed["data"]["indexers"]
            .as_array()
            .expect("indexers array")
            .iter()
            .any(|indexer| indexer["name"] == "Synthetic Budget Indexer"),
        "nothing was created: {listed}"
    );

    let created = gql(&ctx, CREATE_INDEXER, create_input(Value::Null)).await;
    assert_no_errors(&created);
    let id = created["data"]["createIndexerConfig"]["id"]
        .as_str()
        .expect("created indexer id")
        .to_string();
    let refused_update = gql(
        &ctx,
        UPDATE_INDEXER,
        json!({ "input": { "id": id, "maxQueriesPerMinute": 0 } }),
    )
    .await;
    let (message, _) = first_graphql_error_message_and_code(&refused_update);
    assert!(
        message.contains("max queries per minute"),
        "unexpected error: {refused_update}"
    );
    assert_eq!(stored_budget(&ctx, &id).await, Value::Null);
}
