use super::*;

#[tokio::test]
async fn discovery_browse_sections_serialize_supported_surfaces() {
    let ctx = TestContext::new().await;
    for section_id in ["browse_all", "browse_genre_drama"] {
        let body = gql(
            &ctx,
            r#"query($input: DiscoveryItemsInput!) {
                discoveryItems(input: $input) {
                    totalCount
                    section { sectionId surface totalCount }
                }
            }"#,
            json!({"input": {"sectionId": section_id, "limit": 1}}),
        )
        .await;
        assert_no_errors(&body);
        let section = &body["data"]["discoveryItems"]["section"];
        assert_eq!(section["sectionId"], section_id);
        assert_eq!(section["surface"], "PUBLIC");
    }
}
