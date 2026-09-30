//! A withheld list provider is off the whole Lists surface: the catalog does
//! not show it, and no follow or preview can name it, by source or by link.

use std::collections::BTreeMap;

use super::*;
use crate::lists::catalog::{LIST_PROVIDER_IMDB, LIST_SOURCE_TYPE_IMDB_USER_LIST};
use crate::lists::public::{ListSourceDraft, PublicListInput};

fn list_manager() -> User {
    let mut manager = User::new_admin("list-manager");
    manager.authorization = scryer_domain::UserAuthorization {
        app: AppPermissionMask::MANAGE_LISTS,
        loaded: true,
        ..Default::default()
    };
    manager
}

async fn lists_harness() -> MediaRequestTestHarness {
    let harness = bootstrap_media_request_app();
    super::list_experimental_gate::set_experimental_features(&harness, true).await;
    harness
}

fn refused_as_withheld<T: std::fmt::Debug>(result: AppResult<T>) {
    match result {
        Err(AppError::Validation(message)) => assert_eq!(
            message, "IMDb lists are not available in Scryer right now",
            "unexpected refusal"
        ),
        other => panic!("expected the IMDb refusal, got {other:?}"),
    }
}

const IMDB_LIST_LINK: &str = "https://www.imdb.com/list/ls000000123/";

fn imdb_list_params() -> BTreeMap<String, String> {
    BTreeMap::from([(
        scryer_domain::LIST_SOURCE_IMDB_LIST_ID_PARAM.to_string(),
        "ls000000123".to_string(),
    )])
}

#[tokio::test]
async fn the_catalog_offers_no_imdb_provider() {
    let harness = lists_harness().await;
    let manifests = harness
        .app
        .list_provider_catalog(&list_manager())
        .await
        .expect("catalog");
    assert!(
        manifests
            .iter()
            .all(|manifest| manifest.provider_type != LIST_PROVIDER_IMDB),
        "IMDb is withheld: {:?}",
        manifests
            .iter()
            .map(|manifest| &manifest.provider_type)
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn imdb_sources_and_links_cannot_be_followed_or_previewed() {
    let harness = lists_harness().await;
    let manager = list_manager();
    let app = &harness.app;

    refused_as_withheld(
        app.subscribe_public_list(
            &manager,
            PublicListInput {
                provider: Some(LIST_PROVIDER_IMDB.to_string()),
                source_type: Some(LIST_SOURCE_TYPE_IMDB_USER_LIST.to_string()),
                params: imdb_list_params(),
                ..Default::default()
            },
        )
        .await,
    );
    refused_as_withheld(
        app.subscribe_public_list(
            &manager,
            PublicListInput {
                provider: Some(LIST_PROVIDER_IMDB.to_string()),
                source_type: Some("smg_chart:imdb.top250.movies:global".to_string()),
                ..Default::default()
            },
        )
        .await,
    );
    refused_as_withheld(
        app.subscribe_public_list(
            &manager,
            PublicListInput {
                url: Some(IMDB_LIST_LINK.to_string()),
                ..Default::default()
            },
        )
        .await,
    );
    refused_as_withheld(app.preview_list_url(&manager, IMDB_LIST_LINK).await);
    refused_as_withheld(
        app.preview_list_source(
            &manager,
            ListSourceDraft {
                provider: Some(LIST_PROVIDER_IMDB.to_string()),
                source_type: Some("smg_chart:imdb.toptv.series:global".to_string()),
                ..Default::default()
            },
        )
        .await,
    );
    refused_as_withheld(
        app.preview_list_source(
            &manager,
            ListSourceDraft {
                provider: Some(LIST_PROVIDER_IMDB.to_string()),
                source_type: Some(LIST_SOURCE_TYPE_IMDB_USER_LIST.to_string()),
                params: imdb_list_params(),
                ..Default::default()
            },
        )
        .await,
    );

    assert!(
        harness.lists.subscriptions.lock().unwrap().is_empty(),
        "nothing was followed"
    );
}

#[tokio::test]
async fn an_imdb_list_followed_earlier_is_kept_but_not_previewed() {
    let harness = lists_harness().await;
    let mut followed = crate::lists::test_support::subscription("public-imdb-list");
    followed.source.provider = LIST_PROVIDER_IMDB.to_string();
    followed.source.source_type = LIST_SOURCE_TYPE_IMDB_USER_LIST.to_string();
    followed.source.params = imdb_list_params();
    followed.source.origin = scryer_domain::ListSourceOrigin::SmgImdbList;
    *harness.lists.subscriptions.lock().unwrap() = vec![followed.clone()];

    refused_as_withheld(
        harness
            .app
            .preview_public_list(&list_manager(), "public-imdb-list")
            .await,
    );

    let stored = harness.lists.subscription("public-imdb-list");
    assert_eq!(stored.source, followed.source, "the follow is untouched");
    assert_eq!(stored.enabled, followed.enabled);
}
