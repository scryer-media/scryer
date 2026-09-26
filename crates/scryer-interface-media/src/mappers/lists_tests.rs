use super::*;

fn filter_input(kind: ListFilterKindValue) -> ListFilterInput {
    ListFilterInput {
        kind,
        scale: None,
        value: None,
        from: None,
        to: None,
        values: Vec::new(),
    }
}

fn subscribe_input(scope: ListScopeValue) -> SubscribeListInput {
    SubscribeListInput {
        scope,
        provider: Some("fixture".to_string()),
        source_type: Some("chart".to_string()),
        params: vec![ListParamInput {
            key: " region ".to_string(),
            value: "north".to_string(),
        }],
        url: None,
        name: None,
        kinds: None,
        mode: ListModeValue::Add,
        routes: vec![ListRouteInput {
            kind: MediaFacetValue::Movie,
            library_id: ID::from("library-a"),
            quality_profile_id: Some(ID::from(" ")),
            root_folder_id: None,
            monitor_type: "movie".to_string(),
            min_availability: None,
            use_season_folders: None,
            release_numbering: None,
            tags: vec!["fixture-tag".to_string()],
        }],
        filters: vec![filter_input(ListFilterKindValue::ReleasedOnly)],
        max_per_sync: Some(5),
        on_leave: ListOnLeaveValue::Log,
    }
}

#[test]
fn filters_round_trip_through_their_graphql_shape() {
    let filters = vec![
        ListFilter::RatingAtLeast {
            scale: "ten".to_string(),
            value: 7.5,
        },
        ListFilter::ReleaseYear {
            from: Some(2030),
            to: None,
        },
        ListFilter::ExcludeGenres {
            genres: vec!["fixture-genre".to_string()],
        },
        ListFilter::Format {
            formats: vec!["fixture-format".to_string()],
        },
        ListFilter::Language {
            languages: vec!["xx".to_string()],
        },
        ListFilter::ReleasedOnly,
        ListFilter::NotSequelWithoutBase,
    ];
    for filter in filters {
        let payload = from_filter(filter.clone());
        let input = ListFilterInput {
            kind: payload.kind,
            scale: payload.scale,
            value: payload.value,
            from: payload.from,
            to: payload.to,
            values: payload.values,
        };
        assert_eq!(filter_from_input(input).expect("valid filter"), filter);
    }
}

#[test]
fn a_rating_filter_without_a_scale_or_value_is_refused() {
    assert!(filter_from_input(filter_input(ListFilterKindValue::RatingAtLeast)).is_err());
    let mut missing_value = filter_input(ListFilterKindValue::RatingAtLeast);
    missing_value.scale = Some("ten".to_string());
    assert!(filter_from_input(missing_value).is_err());
}

#[test]
fn a_personal_origin_carries_its_kind_only() {
    let personal = from_media_request_origin(
        &MediaRequestOrigin::PersonalList {
            subscription_id: "sub-personal".to_string(),
        },
        Some("Fixture Personal".to_string()),
    );
    assert!(personal.kind == MediaRequestOriginKindValue::PersonalList);
    assert!(personal.public_subscription_id.is_none());
    assert!(personal.public_list_name.is_none());

    let public = from_media_request_origin(
        &MediaRequestOrigin::PublicList {
            subscription_id: "sub-public".to_string(),
        },
        Some("Fixture Chart".to_string()),
    );
    assert!(public.kind == MediaRequestOriginKindValue::PublicList);
    assert_eq!(
        public.public_subscription_id.map(|id| id.to_string()),
        Some("sub-public".to_string())
    );
    assert_eq!(public.public_list_name.as_deref(), Some("Fixture Chart"));
}

#[test]
fn only_a_public_follow_maps_and_blank_ids_read_as_unset() {
    assert!(public_list_input_from_input(subscribe_input(ListScopeValue::Personal)).is_err());
    let input = public_list_input_from_input(subscribe_input(ListScopeValue::Public))
        .expect("public follow");
    assert_eq!(
        input.params.get("region").map(String::as_str),
        Some("north")
    );
    assert_eq!(input.routes[0].quality_profile_id, None);
    assert_eq!(input.routes[0].tags, vec!["fixture-tag".to_string()]);
    assert_eq!(input.max_per_sync, Some(5));
}

#[test]
fn an_edit_distinguishes_an_omitted_cap_from_a_lifted_one() {
    let patch = |max_per_sync| UpdateListSubscriptionInput {
        name: None,
        kinds: None,
        mode: None,
        routes: None,
        filters: None,
        max_per_sync,
        on_leave: None,
    };
    let omitted = public_list_patch_from_input(patch(MaybeUndefined::Undefined)).expect("patch");
    assert_eq!(omitted.max_per_sync, None);
    let lifted = public_list_patch_from_input(patch(MaybeUndefined::Null)).expect("patch");
    assert_eq!(lifted.max_per_sync, Some(None));
    let set = public_list_patch_from_input(patch(MaybeUndefined::Value(3))).expect("patch");
    assert_eq!(set.max_per_sync, Some(Some(3)));
    assert!(public_list_patch_from_input(patch(MaybeUndefined::Value(-1))).is_err());
}

#[test]
fn a_list_exclusion_needs_its_list() {
    let input = AddListExclusionInput {
        kind: MediaFacetValue::Movie,
        external_ids: vec![ExternalIdInput {
            source: "tmdb".to_string(),
            kind: None,
            value: "900001".to_string(),
        }],
        display_title: "Fixture Title".to_string(),
        year: Some(2031),
        scope: ListExclusionScopeValue::List,
        subscription_id: None,
    };
    assert!(list_exclusion_input_from_input(input.clone()).is_err());
    let scoped = list_exclusion_input_from_input(AddListExclusionInput {
        subscription_id: Some(ID::from("sub-public")),
        ..input
    })
    .expect("scoped exclusion");
    assert_eq!(
        scoped.scope,
        ListExclusionScope::List {
            subscription_id: "sub-public".to_string()
        }
    );
}
