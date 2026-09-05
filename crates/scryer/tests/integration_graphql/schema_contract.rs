use super::*;

#[tokio::test]
async fn graphql_get_returns_non_500() {
    let ctx = TestContext::new().await;
    let resp = ctx
        .http_client()
        .get(format!("{}/graphql", ctx.app_url))
        .send()
        .await
        .unwrap();
    // GET on a POST-only endpoint — should not crash
    assert_ne!(resp.status().as_u16(), 500);
}

// ---------------------------------------------------------------------------
// Introspection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn graphql_introspection_query_type() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ __schema { queryType { name } } }", json!({})).await;
    assert_eq!(body["data"]["__schema"]["queryType"]["name"], "QueryRoot");
}

#[tokio::test]
async fn graphql_introspection_mutation_type() {
    let ctx = TestContext::new().await;
    let body = gql(&ctx, "{ __schema { mutationType { name } } }", json!({})).await;
    assert_eq!(
        body["data"]["__schema"]["mutationType"]["name"],
        "MutationRoot"
    );
}

fn graphql_description_is_blank(value: &Value) -> bool {
    value
        .as_str()
        .is_none_or(|description| description.trim().is_empty())
}

fn graphql_description_contains_em_dash(value: &Value) -> bool {
    value
        .as_str()
        .is_some_and(|description| description.contains('\u{2014}'))
}

fn record_graphql_description(
    missing: &mut Vec<String>,
    em_dashes: &mut Vec<String>,
    path: String,
    description: &Value,
) {
    if graphql_description_is_blank(description) {
        missing.push(path);
    } else if graphql_description_contains_em_dash(description) {
        em_dashes.push(path);
    }
}

#[tokio::test]
async fn graphql_http_schema_is_fully_documented() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          __schema {
            queryType { name }
            mutationType { name }
            types {
              kind
              name
              description
              fields(includeDeprecated: true) {
                name
                description
                isDeprecated
                deprecationReason
                args(includeDeprecated: true) {
                  name
                  description
                  isDeprecated
                  deprecationReason
                  type {
                    kind
                    name
                    ofType {
                      kind
                      name
                      ofType {
                        kind
                        name
                        ofType { kind name }
                      }
                    }
                  }
                }
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                      ofType { kind name }
                    }
                  }
                }
              }
              interfaces { name }
              inputFields(includeDeprecated: true) {
                name
                description
                isDeprecated
                deprecationReason
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                      ofType { kind name }
                    }
                  }
                }
              }
              enumValues(includeDeprecated: true) {
                name
                description
                isDeprecated
                deprecationReason
              }
              possibleTypes { name }
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let schema = &body["data"]["__schema"];
    let query_root = schema["queryType"]["name"]
        .as_str()
        .expect("GraphQL query root name");
    let mutation_root = schema["mutationType"]["name"]
        .as_str()
        .expect("GraphQL mutation root name");
    let type_by_name: BTreeMap<&str, &Value> = schema["types"]
        .as_array()
        .expect("GraphQL schema types")
        .iter()
        .filter_map(|ty| ty["name"].as_str().map(|name| (name, ty)))
        .collect();

    let built_in_scalars = ["Boolean", "Float", "ID", "Int", "String"];
    let mut pending = vec![query_root.to_string(), mutation_root.to_string()];
    let mut visited = std::collections::BTreeSet::new();
    let mut missing = Vec::new();
    let mut em_dashes = Vec::new();

    while let Some(type_name) = pending.pop() {
        if type_name == "SubscriptionRoot" || type_name.starts_with("__") {
            continue;
        }
        if !visited.insert(type_name.clone()) {
            continue;
        }
        let ty = type_by_name
            .get(type_name.as_str())
            .unwrap_or_else(|| panic!("reachable GraphQL type {type_name} must exist"));
        let kind = ty["kind"].as_str().expect("GraphQL type kind");
        let is_root = type_name == query_root || type_name == mutation_root;
        let is_built_in_scalar = kind == "SCALAR" && built_in_scalars.contains(&type_name.as_str());

        if !is_root && !is_built_in_scalar {
            record_graphql_description(
                &mut missing,
                &mut em_dashes,
                format!("type {type_name}"),
                &ty["description"],
            );
        }

        match kind {
            "OBJECT" | "INTERFACE" => {
                for field in ty["fields"]
                    .as_array()
                    .unwrap_or_else(|| panic!("fields for GraphQL type {type_name}"))
                {
                    let field_name = field["name"].as_str().expect("GraphQL field name");
                    let field_path = format!("{type_name}.{field_name}");
                    record_graphql_description(
                        &mut missing,
                        &mut em_dashes,
                        field_path.clone(),
                        &field["description"],
                    );
                    if field["isDeprecated"].as_bool() == Some(true) {
                        record_graphql_description(
                            &mut missing,
                            &mut em_dashes,
                            format!("{field_path} deprecation reason"),
                            &field["deprecationReason"],
                        );
                    }
                    for argument in field["args"]
                        .as_array()
                        .unwrap_or_else(|| panic!("arguments for GraphQL field {field_path}"))
                    {
                        let argument_name =
                            argument["name"].as_str().expect("GraphQL argument name");
                        record_graphql_description(
                            &mut missing,
                            &mut em_dashes,
                            format!("{field_path}({argument_name}:)"),
                            &argument["description"],
                        );
                        if argument["isDeprecated"].as_bool() == Some(true) {
                            record_graphql_description(
                                &mut missing,
                                &mut em_dashes,
                                format!("{field_path}({argument_name}:) deprecation reason"),
                                &argument["deprecationReason"],
                            );
                        }
                        if let Some(argument_type) = graphql_type_leaf_name(&argument["type"])
                            && !built_in_scalars.contains(&argument_type)
                        {
                            pending.push(argument_type.to_string());
                        }
                    }
                    if let Some(field_type) = graphql_type_leaf_name(&field["type"])
                        && !built_in_scalars.contains(&field_type)
                    {
                        pending.push(field_type.to_string());
                    }
                }
                if let Some(implemented_types) = ty["interfaces"].as_array() {
                    for implemented_type in implemented_types {
                        pending.push(
                            implemented_type["name"]
                                .as_str()
                                .expect("GraphQL interface name")
                                .to_string(),
                        );
                    }
                }
                if kind == "INTERFACE" {
                    for possible_type in ty["possibleTypes"].as_array().unwrap_or_else(|| {
                        panic!("possible types for GraphQL interface {type_name}")
                    }) {
                        pending.push(
                            possible_type["name"]
                                .as_str()
                                .expect("GraphQL interface member name")
                                .to_string(),
                        );
                    }
                }
            }
            "INPUT_OBJECT" => {
                for field in ty["inputFields"]
                    .as_array()
                    .unwrap_or_else(|| panic!("input fields for GraphQL type {type_name}"))
                {
                    let field_name = field["name"].as_str().expect("GraphQL input field name");
                    record_graphql_description(
                        &mut missing,
                        &mut em_dashes,
                        format!("{type_name}.{field_name}"),
                        &field["description"],
                    );
                    if field["isDeprecated"].as_bool() == Some(true) {
                        record_graphql_description(
                            &mut missing,
                            &mut em_dashes,
                            format!("{type_name}.{field_name} deprecation reason"),
                            &field["deprecationReason"],
                        );
                    }
                    if let Some(field_type) = graphql_type_leaf_name(&field["type"])
                        && !built_in_scalars.contains(&field_type)
                    {
                        pending.push(field_type.to_string());
                    }
                }
            }
            "ENUM" => {
                for value in ty["enumValues"]
                    .as_array()
                    .unwrap_or_else(|| panic!("enum values for GraphQL type {type_name}"))
                {
                    let value_name = value["name"].as_str().expect("GraphQL enum value name");
                    let value_path = format!("{type_name}.{value_name}");
                    record_graphql_description(
                        &mut missing,
                        &mut em_dashes,
                        value_path.clone(),
                        &value["description"],
                    );
                    if value["isDeprecated"].as_bool() == Some(true) {
                        record_graphql_description(
                            &mut missing,
                            &mut em_dashes,
                            format!("{value_path} deprecation reason"),
                            &value["deprecationReason"],
                        );
                    }
                }
            }
            "UNION" => {
                for possible_type in ty["possibleTypes"]
                    .as_array()
                    .unwrap_or_else(|| panic!("possible types for GraphQL union {type_name}"))
                {
                    pending.push(
                        possible_type["name"]
                            .as_str()
                            .expect("GraphQL union member name")
                            .to_string(),
                    );
                }
            }
            "SCALAR" => {}
            unexpected => panic!("unsupported reachable GraphQL type kind {unexpected}"),
        }
    }

    missing.sort();
    em_dashes.sort();
    assert!(
        missing.is_empty() && em_dashes.is_empty(),
        "public HTTP GraphQL schema documentation is incomplete\nmissing descriptions ({}):\n{}\ndescriptions containing em dashes ({}):\n{}",
        missing.len(),
        missing.join("\n"),
        em_dashes.len(),
        em_dashes.join("\n"),
    );
}

#[tokio::test]
async fn graphql_introspection_schema_census_matches_contract_baseline() {
    let ctx = TestContext::new().await;
    let sdl = schema_sdl(&ctx);
    assert_eq!(scryer_interface::export_schema_sdl(), sdl);
    assert!(sdl.contains("type QueryRoot"));
    assert!(sdl.contains("type MutationRoot"));
    assert!(sdl.contains("type SubscriptionRoot"));
    assert!(sdl.contains("scalar Date"));
    assert!(sdl.contains("scalar DateTime"));
    assert!(sdl.contains("scalar Long"));

    let body = gql(
        &ctx,
        r#"
        {
          __schema {
            queryType { fields { name } }
            mutationType { fields { name } }
            subscriptionType { fields { name } }
            types { name kind }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let query_field_count = body["data"]["__schema"]["queryType"]["fields"]
        .as_array()
        .expect("query fields")
        .len();
    let mutation_field_count = body["data"]["__schema"]["mutationType"]["fields"]
        .as_array()
        .expect("mutation fields")
        .len();
    let subscription_field_count = body["data"]["__schema"]["subscriptionType"]["fields"]
        .as_array()
        .expect("subscription fields")
        .len();

    let public_types: Vec<&Value> = body["data"]["__schema"]["types"]
        .as_array()
        .expect("schema types")
        .iter()
        .filter(|ty| {
            ty["name"]
                .as_str()
                .is_some_and(|name| !name.starts_with("__"))
        })
        .collect();
    let kind_count = |kind: &str| -> usize {
        public_types
            .iter()
            .filter(|ty| ty["kind"].as_str() == Some(kind))
            .count()
    };
    let query_field_names: Vec<&str> = body["data"]["__schema"]["queryType"]["fields"]
        .as_array()
        .expect("query fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    let mutation_field_names: Vec<&str> = body["data"]["__schema"]["mutationType"]["fields"]
        .as_array()
        .expect("mutation fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    let public_type_names: Vec<&str> = public_types
        .iter()
        .filter_map(|ty| ty["name"].as_str())
        .collect();

    // cutover: the wanted/cutoff/search surface changed — the top-level
    // `wantedItems` became the derived Missing/Upgrades view, the unpaged
    // `cutoffUnmetTitles` query was dropped, the four per-item trigger mutations +
    // `resetWantedItem` were replaced by `triggerAcquisitionSearch` /
    // `cancelAcquisitionSearch` / `acquisitionSearchJob`, and the payloads gained
    // convergence/recency fields (with new enums + the job payload).
    // 0.17.0 re-pinned this census for the intentionally API-breaking release:
    // query fields moved 119 -> 121 with the added Query.episodeById /
    // Query.collectionById id-anchored lookups; the other counts are unchanged.
    // 0.17.0 API surface trim (root wave): removed 5 dead query roots
    // (discoverySyncStatus, libraryScanSession, mediaServerConnection,
    // outboundRateLimitSnapshot, upstreamSchedulerSnapshot), 1 dead mutation
    // (queueReplacementRelease), and the 5 exclusive snapshot payload OBJECT types.
    // 0.17.0 API surface trim (field wave): removed dead output fields inside
    // consumed types plus 4 never-selected OBJECT types (DiscoverySyncRunPayload and the
    // ExternalImportLibrarySetting{Application,Evidence,Value}Payload trio); the trio's
    // exclusive enums (ExternalImportLibrarySetting{Confidence,Disposition,Key}) drop with
    // it. Root-field counts unchanged; OBJECT 259->255, ENUM 80->77, public types 498->491.
    // 0.17.0 semantic waves (slice 5): stringly String fields became real enums
    // (+12 ENUM), and QueueDownloadScopePayload / ProviderConfigFieldValue became unions
    // (+2 UNION with 6 scope + 5 config-value member OBJECT types, +11 OBJECT).
    // Root-field counts unchanged; ENUM 77->89, OBJECT 255->266, public types 491->516.
    // Library catalog filter options add one query and two payload OBJECT types;
    // external subtitle listing and blocklist lookup add two query roots and eight public types
    // (five OBJECT, two INPUT_OBJECT, and one ENUM support types).
    // Catalog bootstrap no longer exposes filesystem reachability as a blocking query.
    // User login suspension adds one mutation and its INPUT_OBJECT.
    // 2026-07-23 hotfix 0.17.1: server-side interactive release-search jobs
    // (start/cancel mutations + poll query) replace the single join-all
    // `searchReleases` round-trip for the UI so per-indexer results stream in.
    // Adds 1 query root, 2 mutation roots, 3 payload OBJECTs
    // (InteractiveReleaseSearch{,Indexer}Payload, CancelInteractiveReleaseSearchPayload)
    // and 2 ENUMs (state + indexer status): query 118->119, mutation 164->166,
    // OBJECT 273->276, ENUM 90->92, public types 527->532.
    // Async Prowlarr discovery adds one status query, one start mutation, and
    // one input object: query 119->120, mutation 166->167, public types 532->533.
    // The kind-neutral externalImportWarmupStatus supersedes
    // externalImportArrSourceWarmupStatus; deprecating the old field hides it
    // from default (includeDeprecated: false) introspection: query 120->119.
    // The direct-path previewManualImportPath query was removed in favor of
    // server-owned manual-import selections: query 119->118.
    // Server-owned manual-import selection and cancellable external-source
    // warmup add two mutation roots: mutation 167->169.
    // The current selection and classification contract adds six payload
    // objects and one enum: OBJECT 276->282, ENUM 92->93, public types 533->540.
    // Cached queue paging and revision sync add two payload objects and one
    // sort enum: OBJECT 282->284, ENUM 93->94, public types 540->543. The new
    // roots replace deprecated fields in default introspection, so root field
    // counts remain unchanged.
    // Explicit indexer download-client mapping adds one query root, one mutation
    // root, and three payload objects: query 118->119, mutation 169->170,
    // OBJECT 284->287, INPUT_OBJECT 153->154, public types 543->547.
    // Episode media availability adds one object and one enum: public types
    // 547->549, OBJECT 287->288, ENUM 94->95.
    // Restore the operator-selected replacement mutation: mutation 170->171.
    // Provider-level indexer/download-client compatibility adds one object so
    // unsaved indexer drafts can use the same server-derived routing contract.
    // Cached title credits add one payload object behind the new `Title.credits`
    // resolver; the field hangs off an existing type, so root counts are
    // unchanged: OBJECT 291->292, public types 560->561.
    // Plugin auto-update settings add one query root, one mutation root, one
    // payload object, and one input object: query 119->120, mutation 175->176,
    // OBJECT 292->293, INPUT_OBJECT 158->159, public types 561->563.
    // The season-scoped panel's `Collection.episodeRecordsTotal` hangs off an
    // existing type, so no census counts change.
    // Dashboard landing page adds the dashboardActivityStats and storageRoots
    // query roots plus their three payload objects
    // (DashboardActivityStatsPayload, ActivityWindowCountsPayload,
    // StorageRootUsagePayload): query 120->122, OBJECT 293->296,
    // public types 563->566.
    // Manual-imports dashboard panel adds PendingImportReasonClassValue beside
    // the free-text reason (plus createdAt/sizeBytes on the existing item
    // payload, which add no types), and recently-imported enrichment adds
    // libraryId/sizeBytes to TitleHistoryEventPayload: ENUM 99->100,
    // public types 566->567. Root-field counts unchanged.
    // Torrent seeding profiles add two query roots, five mutation roots, three
    // payload objects, four inputs, and two enums: query 119->121, mutation
    // 175->180, OBJECT 291->294, INPUT_OBJECT 158->162, ENUM 99->101, public
    // types 560->569.
    // Queue seeding progress adds six nullable fields on DownloadQueueItemPayload
    // and the enum behind one of them: ENUM 101->102, public types 569->570.
    // Root-field and OBJECT counts are unchanged — the fields are additive on an
    // existing type.
    // Post-import handoff adds one enum plus one field on the seeding-profile
    // payload and both of its inputs: ENUM 102->103, public types 570->571.
    // Root-field, OBJECT and INPUT_OBJECT counts are unchanged — the fields are
    // additive on existing types.
    // (Merged with release-0.18.17: both branches' additions above are
    // additive from the shared 119/175/291/158/99/560 base.)
    // Batched rename preview adds the mediaRenamePreviewBulk query root and its
    // input object, reusing the existing plan payload: query 124->125,
    // INPUT_OBJECT 163->164, public types 578->579.
    // Renaming as a background job adds the renameTitles mutation root, its
    // input object and payload, and a TITLE_RENAME value on the existing job
    // key enum: mutation 181->182, OBJECT 299->300, INPUT_OBJECT 164->165,
    // public types 579->581. ENUM is unchanged: the value joins an enum that
    // already exists.
    // Deprecating applyMediaRename and applyMediaRenameBulk in favour of that
    // job drops both from this count: introspection omits deprecated fields
    // unless asked for them, so the census measures live surface. The fields
    // and their input types are still served: mutation 182->180.
    // Minimum-seeder admission adds the setMinimumSeedersFloor mutation root
    // and its input object: mutation 180->181, INPUT_OBJECT 165->166, public
    // types 581->582. The threshold itself rides on existing types as
    // SeedingProfilePayload.minimumSeeders and
    // DefaultSeedingProfilePayload.minimumSeedersFloor, so OBJECT is unchanged.
    // Login-factor verification then adds five mutations for enrollment and
    // passkey/TOTP completion: mutation 181->186, public types 582->585,
    // OBJECT 300->301, and INPUT_OBJECT 166->168.
    // Temporary-password replacement adds one mutation and its input object:
    // mutation 186->187, INPUT_OBJECT 168->169, public types 585->586.
    // Durable indexer HTTP error history adds the indexerErrors and indexerError
    // query roots: query 127->129. The title-id work changes existing metadata
    // query inputs and payload fields only, so it adds no roots or named types.
    // OAuth client registrations add their create, update, and delete mutation
    // roots and six named types: mutation 187->190, public types 593->599.
    // API-key lifecycle adds two queries, two mutations, and five named types:
    // query 129->131, mutation 190->192, public types 599->604.
    // Application upgrade status adds one query root, one payload object, and
    // two enum types: query +1, OBJECT +1, ENUM +2, public types +3.
    // Starting an application upgrade adds one mutation root, its input, and
    // its acceptance payload. The status run fields reuse JobRunPayload and
    // APPLICATION_UPGRADE joins the existing JobKeyValue enum: mutation +1,
    // OBJECT +1, INPUT_OBJECT +1, public types +2.
    // Media-server playback links add one object behind existing title, episode,
    // and calendar payloads: OBJECT 314->315, public types 609->610.
    // Query, mutation, subscription, input-object, and enum counts are unchanged.
    // Manual-import video facts add one object: OBJECT 315->316, public types 610->611.
    // Live import activity adds one query, one mutation, one subscription, two
    // payload objects, and one phase enum: query 132->133, mutation 193->194,
    // subscription 13->14, OBJECT 316->318, ENUM 110->111, public types 611->614.
    // Local movie-entity detail adds one query. In the combined release schema,
    // its payload graph also makes one existing enum reachable: query 133->134,
    // ENUM 111->112, public types 614->615.
    // Plugin config preset overrides add one key/value payload object behind the
    // existing config option type: OBJECT 318->319, public types 615->616.
    // Account-security reauthentication adds three mutation roots without
    // changing the established factor-mutation payload contracts: mutation 194->197.
    // The maintenance-rule authoring surface adds four query roots (list, get,
    // revisions, action catalog) and six mutation roots (create, matcher edit,
    // metadata edit, delete, validate, preview), with nine payload objects, six
    // inputs, and six enums: query 134->138, mutation 197->203, OBJECT 319->328,
    // INPUT_OBJECT 173->179, ENUM 112->118, public types 616->637.
    // The maintenance dark evaluator adds four query roots (candidates,
    // evaluation runs, instance gates, exclusions) and five mutation roots
    // (rule mode, instance gates, exclude, remove exclusion, run now), with six
    // payload objects, three inputs, and one candidate-state enum:
    // query 138->142, mutation 203->208, OBJECT 328->334, INPUT_OBJECT
    // 179->182, ENUM 118->119, public types 637->647. MAINTENANCE_RULE_EVALUATION
    // joins the existing job key enum, so it adds no type.
    // The maintenance action executor adds the maintenanceActionRuns query
    // root, the setMaintenanceRuleArming and runMaintenanceActionHandlerNow
    // mutation roots, the action-run payload object, the arming input, and the
    // arming enum: query 142->143, mutation 208->210, OBJECT 334->335,
    // INPUT_OBJECT 182->183, ENUM 119->120, public types 647->650.
    // LIFECYCLE_ACTION_HANDLING joins the existing job key enum.
    // Media-server watch-signal sync adds no GraphQL surface of its own:
    // MEDIA_SERVER_SIGNAL_SYNC joins the existing job key enum, so every count
    // below is unchanged.
    //
    // ── The library-location feature line, merged in beside the maintenance
    // line above. Its paragraphs below narrate that branch's own history, so
    // their running totals are the location branch's totals; the asserted
    // numbers at the bottom are the union of both branches over the shared
    // base (query 134 + 9 maintenance + 7 location = 150; mutation 197 + 13 +
    // 5 = 215; OBJECT 319 + 16 + 50 = 385; INPUT_OBJECT 173 + 10 + 10 = 193;
    // ENUM 112 + 8 + 20 = 140; public types 616 + 34 + 80 = 730). ──
    //
    // Folder-match correction adds the changeTitleFolderPreview query root and
    // the applyTitleFolderChange mutation root, four payload objects (preview,
    // apply, title ref, displaced-title repair), two inputs, and three enums
    // (ownership state, resolution, outcome): query +1, mutation +1, OBJECT +4,
    // INPUT_OBJECT +2, ENUM +3, public types +9.
    // The stated baseline had also drifted one feature behind before this change
    // — an earlier query root, mutation root, payload object, input object, and
    // enum landed without updating these numbers — so the totals below absorb
    // that drift as well: query 134->136, mutation 197->199, OBJECT 319->324,
    // INPUT_OBJECT 173->176, ENUM 112->116, public types 616->628.
    // Root-move location operations add the locationOperationPreview and
    // locationOperation query roots and the startLocationOperation /
    // cancelLocationOperation / resumeLocationOperation mutation roots, with
    // seventeen payload objects (preview, plan counts, per-kind count, plan
    // section, plan item, selection classification, classification group,
    // classified title, free-space estimate, verification statement, plan
    // confirmation, operation, operation counters, title checkpoint, start,
    // cancel, resume), three inputs (destination, preview, start), and seven
    // enums (operation type, execution mode, operation state, title class, plan
    // item kind, checkpoint state, confirmation requirement): query 136->138,
    // mutation 199->202, OBJECT 324->341, INPUT_OBJECT 176->179, ENUM 116->123,
    // public types 628->655. The LOCATION_OPERATION job key joins the existing
    // JobKeyValue enum, so it adds no type.
    // The dedup/rename asset listing (T090, FR-091, US8.1/US8.4) adds the
    // locationOperationAssets query root and four payload objects (the listing,
    // one title's assets, one renamed asset, one deduplicated asset): query
    // 138->139, OBJECT 352->356, public types 673->677. The per-title state
    // reuses the existing LocationTitleCheckpointStateValue, so ENUM is
    // unchanged, and nothing is added to the operation payload itself: the
    // per-file identities live in the stored plan, which a progress poll has no
    // reason to load.
    // The two root-scoped workflows (T064, US4 and US5, FR-020 to FR-029) add
    // the locationRootChangePreview and locationRootConsolidationPreview query
    // roots, thirteen payload objects (root-change preview, consolidation
    // preview, title accounting, blocked title, root identity retention,
    // content inventory, content bucket, content entry, sampled paths,
    // retirement contract, retirement blocker, consolidation classification,
    // default-root transfer), four inputs (the two preview inputs and the two
    // start targets), and one enum for FR-027's three content classes:
    // query 139->141, OBJECT 356->369, INPUT_OBJECT 179->183, ENUM 131->132,
    // public types 678->696. No new mutation: both workflows confirm through
    // the existing startLocationOperation, whose input gained the two
    // root-scoped destination variants beside the selection it already carried.
    //
    // FR-020 is one settings action with two destinations, and the surface was
    // folded to match it: `locationRootChangePreview` and
    // `locationRootConsolidationPreview` became one `locationRootScopePreview`
    // (query 150->149), and the four inputs became two —
    // `LocationRootScopePreviewInput` and `LocationRootScopeTargetInput`
    // replace `LocationRootChangePreviewInput`,
    // `LocationRootConsolidationPreviewInput`, `LocationRootChangeTargetInput`
    // and `LocationRootConsolidationTargetInput` (INPUT_OBJECT 191->189,
    // public types 727->725). No object and no enum changed:
    // `LocationRootScopePreviewPayload` was already one payload for both
    // branches, and `StartLocationOperationInput` swapped its `rootChange` and
    // `rootConsolidation` fields for one `rootScope`.
    // OAuth-bound Jellyfin account linking (merged from main) adds one mutation
    // root and reuses the existing linked-account payload: mutation +1, and its
    // main-side census (OBJECT +1, INPUT_OBJECT +1) rides along.
    // ── 0.19.9 through 0.19.11, merged from the release line on top of the
    // totals above (the OAuth-bound Jellyfin link was already counted). ──
    // Multi-episode file deletion adds the deleteEpisodeFilesPreview query and
    // the deleteEpisodeFiles mutation, three payload objects, and two inputs:
    // query +1, mutation +1, OBJECT +3, INPUT_OBJECT +2, public types +5.
    // Advanced monitoring adds the season/series-movie selection payload pair,
    // its input pair, and the anime-movie metadata payload behind the existing
    // metadata series type: OBJECT +3, INPUT_OBJECT +2, public types +5.
    // Query, mutation, and subscription roots are unchanged; ADVANCED joins the
    // existing MonitorTypeValue enum.
    // Stored OAuth client kinds add one enum behind the existing registration
    // payload and create input: ENUM +1, public types +1.
    // Totals: query 149->150, mutation 219->220, OBJECT 381->387,
    // INPUT_OBJECT 194->198, ENUM 139->140, public types 726->737.
    // The srrdb filename recovery switch is an additive field on the existing
    // general settings payload and update input, so no census count moves.
    // The instance-wide feature switches add the actor-only `instanceFeatures`
    // query and its `InstanceFeaturesPayload`: query 150->151, OBJECT 387->388,
    // public types 737->738. The two switches themselves are additive fields on
    // the existing general settings payload and update input, so INPUT_OBJECT,
    // ENUM, mutation, and subscription counts are unchanged.
    // Admin-defined title tags add the `titleTagDefinitions` registry read,
    // which any authenticated caller may make because the tag picker and the
    // catalog filter both need the vocabulary: query 151->152.
    // Request rules (spec 0003 section 7) add nine query roots: the three
    // authoring reads (`requestRuleSets`, `requestRuleSet`,
    // `requestRuleRevisions`), the instance gate, the two decision reads
    // (`requestRuleDecision`, `requestRuleDecisions`), the Rules Context
    // Reference document, the requester pre-flight, and `titleClaims`.
    // Query 151->160.
    assert_eq!(
        query_field_count, 161,
        "query fields: {query_field_names:?}"
    );
    // First-class proxies (WP4) add one mutation, resetProxyHostKey: SSH host
    // keys are pinned on trust-on-first-use, so a legitimate server rekey needs
    // an explicit operator-driven way to forget the pin. Every other proxy and
    // download-client change in that work package is additive fields on types
    // that already existed. 215->216.
    // Indexer search (spec 0002) adds two mutations on the interactive-search
    // job: issueInteractiveReleaseCandidateToken and queueUnlinkedRelease.
    // 216->218.
    // Admin-defined title tags add four mutations: the per-title patch
    // (updateTitleTags) plus the three registry writes beside the delay
    // profiles (create/update/deleteTitleTagDefinition). 220->224.
    // Series-movie tags add a fifth, updateSeriesMovieTags, beside
    // setSeriesMovieMonitored: a series movie is a link row rather than a
    // title, so its tag patch takes link ids and cannot ride updateTitleTags.
    // 224->225.
    // Request rules add eleven mutations: six authoring roots (create, matcher
    // edit, metadata edit, mode, delete, validate), the author-side preview,
    // the instance gate, and the three administrator claim operations.
    // 220->231.
    assert_eq!(
        mutation_field_count, 236,
        "mutation fields: {mutation_field_names:?}"
    );
    // Cross-library transfer (T082, FR-055/FR-056) surfaces destination-title
    // detection on the existing classified-title payload: five additive fields
    // and one new enum for the match outcome (unique, none, ambiguous,
    // same-name-without-identity), so ENUM 123->124 and public types 655->656.
    // No new object, input, query, or mutation.
    // Series↔anime facet conversion (T083, FR-057/FR-058) adds one field on the
    // same classified-title payload plus the conversion payload, its per-setting
    // payload, and the setting-disposition enum (becomes invalid, resets,
    // changes meaning): OBJECT 341->343, ENUM 124->125, public types 656->659.
    // The FR-060/FR-062 link and collection dispositions ride the existing plan
    // items behind new reason codes, so they add no type.
    // Merging into an existing destination title (T085, US7, FR-063 to FR-071)
    // adds the FR-071 preview summary on the existing preview payload and the
    // named candidates behind the ids-only ambiguous list. Nine payload objects
    // (merge preview, blocked record, destination-wins entry, table
    // disposition, role change, reserved-tag conflict, media-request repoint,
    // dropped category, ambiguous candidate) and five enums (disposition,
    // block reason, media role, role-change reason, post-merge work):
    // OBJECT 343->352, ENUM 125->130, public types 659->673. No new query,
    // mutation, or input: `merges` and `ambiguousDestinationCandidates` are
    // additive fields on payloads that already exist.
    // Retiring the direct root write (T093, FR-077/SC-009) deprecates the
    // TitleOptionsInput.rootFolderId input field. Deprecated input fields drop
    // out of default introspection the way deprecated output fields do, but
    // this census counts types and root fields only, so every count below is
    // unchanged.
    // Reaching the "files are already there" adoption engine (T052, US3,
    // FR-050 to FR-053) adds one enum, LocationExecutionModeInput, and an
    // optional `mode` field on each of the two existing location inputs:
    // ENUM 130->131, public types 677->678. The requestable enum is narrower
    // than the reported LocationExecutionModeValue on purpose: CATALOG_ONLY is
    // derived from a fileless selection (FR-076) and is never requestable.
    // Query, mutation, subscription, OBJECT, and INPUT_OBJECT counts are
    // unchanged: both fields join input objects that already exist.
    // Descoping the merge engine to "media file records and history, everything
    // else retires with the title" (FR-063 to FR-071) removes five merge payload
    // objects (table disposition, reserved-tag conflict, media-request repoint,
    // dropped category, destination-wins entry) and two merge enums (disposition,
    // post-merge work); folding US4's root change and US5's consolidation onto
    // one root-scoped planner replaces their two preview payloads with one
    // (`LocationRootScopePreviewPayload`): OBJECT 385->379, ENUM 140->138,
    // public types 730->722.
    // Folding the surface to match — one `locationRootScopePreview` query and
    // one `rootScope` start target in place of the change/consolidation pair —
    // then removes two inputs: `LocationRootScopePreviewInput` and
    // `LocationRootScopeTargetInput` replace the four
    // `LocationRootChange*`/`LocationRootConsolidation*` inputs, so
    // INPUT_OBJECT 193->191 and public types 722->720. Query fields drop by one
    // (150->149) with the second preview root; OBJECT and ENUM are unchanged.
    assert_eq!(subscription_field_count, 14);
    // Indexer search (spec 0002): the query subject on the interactive-search
    // job and the unlinked grab add the types below on top of the proxies
    // census. Combined with the location-surface fold above, the totals are
    // public types 720->724, OBJECT 379->380, INPUT_OBJECT 191->193, and
    // ENUM 138->139.
    // Admin-defined title tags add four objects (the definition, the rewrite
    // counts, and the mutation and deletion payloads that carry them) and three
    // inputs (create, update, and the per-title tag patch): OBJECT 388->392,
    // INPUT_OBJECT 198->201, public types 738->745. `TitleCatalogFilterInput.tags`
    // and the `updateTitleTags` result are additive on types that already
    // exist, and no enum joins the schema.
    // Series-movie tags and the maintenance tag actions add one input,
    // UpdateSeriesMovieTagsInput: INPUT_OBJECT 201->202, public types 745->746.
    // Everything else in that work is additive on types that already exist -
    // `tags` on the series-movie link payload, the action spec and the action
    // input, `seriesMovieCount` on the tag definition, `seriesMovies` on the
    // rewrite counts, `requiresTags` on the action descriptor - and the two new
    // action kinds are values inside the existing MaintenanceActionKind enum,
    // so OBJECT and ENUM are unchanged.
    // Request rules add fourteen objects (rule set, revision, detail, delete
    // payload, validation payload, reason, vote, decision, author preview,
    // requester pre-flight, instance gates, title claim, media-request lease,
    // and the request's submit-time metadata), eleven inputs (six authoring,
    // the preview pair, the gate, and the three claim operations), and six
    // enums (evaluation mode, decision outcome, vote, and the three lifecycle
    // claim enums): OBJECT 388->402, INPUT_OBJECT 198->209, ENUM 140->146,
    // public types 738->769. The additive fields on the existing media-request
    // payload and its three inputs add no type.
    // Provider config schemas add the field-condition payload (visibleWhen /
    // requiredWhen on a plugin config field) and its operator enum:
    // OBJECT 406->407, ENUM 146->147, public types 777->779. The `advanced`
    // flag is an additive field on the existing config-field payload.
    assert_eq!(public_types.len(), 779);
    assert_eq!(kind_count("OBJECT"), 407);
    assert_eq!(kind_count("INPUT_OBJECT"), 213);
    assert_eq!(kind_count("ENUM"), 147);
    assert_eq!(kind_count("SCALAR"), 10);
    assert_eq!(kind_count("UNION"), 2);
    assert!(query_field_names.contains(&"backupSettings"));
    assert!(query_field_names.contains(&"proxyConfigs"));
    assert!(query_field_names.contains(&"indexerDownloadClientMappingCatalog"));
    assert!(query_field_names.contains(&"externalImportSetupSecretDraft"));
    assert!(query_field_names.contains(&"externalImportSetupSecretDraftStatus"));
    assert!(query_field_names.contains(&"indexerErrors"));
    assert!(query_field_names.contains(&"indexerError"));
    assert!(query_field_names.contains(&"canCreateMyApiKeys"));
    assert!(query_field_names.contains(&"applicationUpgradeStatus"));
    assert!(query_field_names.contains(&"activeImportStreams"));
    assert!(query_field_names.contains(&"changeTitleFolderPreview"));
    assert!(mutation_field_names.contains(&"applyTitleFolderChange"));
    assert!(public_type_names.contains(&"ChangeTitleFolderPreviewPayload"));
    assert!(public_type_names.contains(&"ChangeTitleFolderPayload"));
    assert!(public_type_names.contains(&"DisplacedTitleRepairPayload"));
    assert!(query_field_names.contains(&"locationOperationPreview"));
    assert!(query_field_names.contains(&"locationOperation"));
    assert!(query_field_names.contains(&"locationOperationAssets"));
    assert!(public_type_names.contains(&"LocationOperationAssetListingPayload"));
    assert!(public_type_names.contains(&"LocationOperationTitleAssetsPayload"));
    assert!(public_type_names.contains(&"LocationOperationRenamedAssetPayload"));
    assert!(public_type_names.contains(&"LocationOperationDeduplicatedAssetPayload"));
    assert!(mutation_field_names.contains(&"startLocationOperation"));
    assert!(mutation_field_names.contains(&"cancelLocationOperation"));
    assert!(mutation_field_names.contains(&"resumeLocationOperation"));
    assert!(public_type_names.contains(&"LocationOperationPreviewPayload"));
    assert!(public_type_names.contains(&"LocationOperationPayload"));
    assert!(public_type_names.contains(&"LocationTitleCheckpointPayload"));
    assert!(public_type_names.contains(&"StartLocationOperationInput"));
    assert!(public_type_names.contains(&"TitleLocationClassValue"));
    // US3: the requestable mode is its own enum, reachable from both location
    // inputs, and distinct from the reported LocationExecutionModeValue.
    assert!(public_type_names.contains(&"LocationExecutionModeInput"));
    assert!(public_type_names.contains(&"LocationExecutionModeValue"));
    // US4 and US5: FR-020's one settings action is one query with two
    // destinations, and one payload whose variant-only sections say which
    // destination the server planned.
    assert!(query_field_names.contains(&"locationRootScopePreview"));
    assert!(!query_field_names.contains(&"locationRootChangePreview"));
    assert!(!query_field_names.contains(&"locationRootConsolidationPreview"));
    assert!(public_type_names.contains(&"LocationRootScopePreviewPayload"));
    assert!(public_type_names.contains(&"LocationRootScopePreviewInput"));
    assert!(public_type_names.contains(&"LocationRootScopeTargetInput"));
    assert!(public_type_names.contains(&"LocationTitleAccountingPayload"));
    assert!(public_type_names.contains(&"LocationBlockedTitlePayload"));
    assert!(public_type_names.contains(&"LocationRootIdentityRetentionPayload"));
    assert!(public_type_names.contains(&"LocationRootContentInventoryPayload"));
    assert!(public_type_names.contains(&"LocationRootContentBucketPayload"));
    assert!(public_type_names.contains(&"LocationRootContentEntryPayload"));
    assert!(public_type_names.contains(&"LocationRootContentClassValue"));
    assert!(public_type_names.contains(&"LocationSampledPathsPayload"));
    assert!(public_type_names.contains(&"LocationRootRetirementContractPayload"));
    assert!(public_type_names.contains(&"LocationRootRetirementBlockerPayload"));
    assert!(public_type_names.contains(&"LocationConsolidationClassificationPayload"));
    assert!(public_type_names.contains(&"LocationDefaultRootTransferPayload"));
    assert!(mutation_field_names.contains(&"cancelActiveImport"));
    assert!(mutation_field_names.contains(&"startApplicationUpgrade"));
    assert!(mutation_field_names.contains(&"accountSecurityPasswordVerify"));
    assert!(mutation_field_names.contains(&"accountSecurityPasskeyStart"));
    assert!(mutation_field_names.contains(&"accountSecurityPasskeyComplete"));
    assert!(mutation_field_names.contains(&"linkCurrentOAuthJellyfinAccount"));
    assert!(query_field_names.contains(&"episode"));
    assert!(query_field_names.contains(&"titleCatalogFilterOptions"));
    assert!(!query_field_names.contains(&"catalogHasValidRoot"));
    // 0.17.0 dataloader/dual-mode workstream added the id-anchored lookups.
    assert!(query_field_names.contains(&"episodeById"));
    assert!(query_field_names.contains(&"collectionById"));
    assert!(!query_field_names.contains(&"episodeMediaFiles"));
    assert!(query_field_names.contains(&"runtimeInfo"));
    assert!(query_field_names.contains(&"cutoffUnmetTitlesPage"));
    assert!(query_field_names.contains(&"dashboardActivityStats"));
    assert!(query_field_names.contains(&"storageRoots"));
    assert!(public_type_names.contains(&"DashboardActivityStatsPayload"));
    assert!(public_type_names.contains(&"ActivityWindowCountsPayload"));
    assert!(public_type_names.contains(&"StorageRootUsagePayload"));
    assert!(public_type_names.contains(&"MediaServerPlaybackLinkPayload"));
    assert!(public_type_names.contains(&"PendingImportReasonClassValue"));
    assert!(mutation_field_names.contains(&"clearExternalImportSetupSecretDraft"));
    assert!(mutation_field_names.contains(&"createProxyConfig"));
    assert!(mutation_field_names.contains(&"setIndexerDownloadClientMapping"));
    assert!(query_field_names.contains(&"seedingProfiles"));
    assert!(mutation_field_names.contains(&"createSeedingProfile"));
    assert!(mutation_field_names.contains(&"setIndexerSeedingProfile"));
    assert!(public_type_names.contains(&"SeedingProfilePayload"));
    assert!(public_type_names.contains(&"IndexerDownloadClientMappingCatalogPayload"));
    assert!(mutation_field_names.contains(&"deleteProxyConfig"));
    assert!(mutation_field_names.contains(&"saveExternalImportSetupSecretDraft"));
    assert!(mutation_field_names.contains(&"beginManualImportSelection"));
    assert!(mutation_field_names.contains(&"cancelExternalImportArrSourceWarmup"));
    assert!(mutation_field_names.contains(&"setUserLoginEnabled"));
    assert!(mutation_field_names.contains(&"testProxyConfig"));
    assert!(mutation_field_names.contains(&"updateProxyConfig"));
    assert!(mutation_field_names.contains(&"updateBackupSettings"));
    assert!(public_type_names.contains(&"BackupSettingsPayload"));
    assert!(public_type_names.contains(&"CreateProxyConfigInput"));
    assert!(public_type_names.contains(&"DeleteProxyConfigPayload"));
    assert!(public_type_names.contains(&"ProxyConfigPayload"));
    assert!(public_type_names.contains(&"ProxyTestResultPayload"));
    assert!(public_type_names.contains(&"SaveExternalImportSetupSecretDraftInput"));
    assert!(public_type_names.contains(&"UpdateProxyConfigInput"));
    assert!(public_type_names.contains(&"ExternalImportSetupSecretDraftPayload"));
    assert!(public_type_names.contains(&"RuntimeInfoPayload"));
    assert!(public_type_names.contains(&"RuntimePathStyleValue"));
    assert!(public_type_names.contains(&"ApplicationUpgradeStatusPayload"));
    assert!(public_type_names.contains(&"ApplicationInstallationKindValue"));
    assert!(public_type_names.contains(&"ApplicationUpgradeOwnerValue"));
    assert!(public_type_names.contains(&"UpdateBackupSettingsInput"));
    assert!(public_type_names.contains(&"CutoffUnmetTitlesPagePayload"));
    // interactive-search job + convergence surface is present…
    assert!(query_field_names.contains(&"acquisitionSearchJob"));
    assert!(mutation_field_names.contains(&"triggerAcquisitionSearch"));
    assert!(mutation_field_names.contains(&"cancelAcquisitionSearch"));
    assert!(public_type_names.contains(&"AcquisitionSearchJobPayload"));
    assert!(public_type_names.contains(&"AcquisitionSearchJobStateValue"));
    assert!(public_type_names.contains(&"TriggerAcquisitionSearchInput"));
    assert!(public_type_names.contains(&"ConvergenceStateValue"));
    assert!(public_type_names.contains(&"RecencyLaneValue"));
    assert!(public_type_names.contains(&"WantedKindValue"));
    // 0.17.1 hotfix: streaming interactive release-search job surface.
    assert!(query_field_names.contains(&"interactiveReleaseSearch"));
    assert!(mutation_field_names.contains(&"startInteractiveReleaseSearch"));
    assert!(mutation_field_names.contains(&"cancelInteractiveReleaseSearch"));
    assert!(public_type_names.contains(&"InteractiveReleaseSearchPayload"));
    assert!(public_type_names.contains(&"InteractiveReleaseSearchIndexerPayload"));
    assert!(public_type_names.contains(&"CancelInteractiveReleaseSearchPayload"));
    assert!(public_type_names.contains(&"InteractiveReleaseSearchStateValue"));
    assert!(public_type_names.contains(&"InteractiveReleaseSearchIndexerStatusValue"));
    // …and the retired per-item trigger mutations / unpaged cutoff query / phase
    // enum are gone.
    assert!(!query_field_names.contains(&"cutoffUnmetTitles"));
    assert!(!mutation_field_names.contains(&"triggerTitleWantedSearch"));
    assert!(!mutation_field_names.contains(&"triggerSeasonWantedSearch"));
    assert!(!mutation_field_names.contains(&"triggerWantedSearch"));
    assert!(!mutation_field_names.contains(&"resetWantedItem"));
    assert!(!public_type_names.contains(&"WantedSearchPhaseValue"));
    assert!(!public_type_names.contains(&"TriggerTitleWantedSearchInput"));
    assert!(!public_type_names.contains(&"TriggerSeasonWantedSearchInput"));
    assert!(!public_type_names.contains(&"TriggerWantedSearchInput"));
    assert!(!public_type_names.contains(&"ResetWantedItemPayload"));

    // The maintenance dark-evaluator contract a later web wave is built
    // against; these names are pinned, not incidental.
    assert!(query_field_names.contains(&"maintenanceCandidates"));
    assert!(query_field_names.contains(&"maintenanceEvaluationRuns"));
    assert!(query_field_names.contains(&"maintenanceInstanceGates"));
    assert!(query_field_names.contains(&"maintenanceExclusions"));
    assert!(mutation_field_names.contains(&"setMaintenanceRuleMode"));
    assert!(mutation_field_names.contains(&"setMaintenanceInstanceGates"));
    assert!(mutation_field_names.contains(&"excludeMaintenanceSubject"));
    assert!(mutation_field_names.contains(&"removeMaintenanceExclusion"));
    assert!(mutation_field_names.contains(&"runMaintenanceEvaluationNow"));
    assert!(public_type_names.contains(&"MaintenanceCandidate"));
    assert!(public_type_names.contains(&"MaintenanceCandidateState"));
    assert!(public_type_names.contains(&"MaintenanceEvaluationRun"));
    assert!(public_type_names.contains(&"MaintenanceInstanceGates"));
    assert!(public_type_names.contains(&"MaintenanceExclusion"));
    assert!(public_type_names.contains(&"DeleteMaintenanceExclusionPayload"));
    assert!(public_type_names.contains(&"MaintenanceEvaluationTriggerPayload"));

    // The maintenance action-executor contract the same web wave binds to.
    assert!(query_field_names.contains(&"maintenanceActionRuns"));
    assert!(mutation_field_names.contains(&"setMaintenanceRuleArming"));
    assert!(mutation_field_names.contains(&"runMaintenanceActionHandlerNow"));
    assert!(public_type_names.contains(&"MaintenanceActionRun"));
    assert!(public_type_names.contains(&"MaintenanceEffectArming"));
    assert!(public_type_names.contains(&"SetMaintenanceRuleArmingInput"));

    // 0.17.0 API surface trim (root wave): dead root fields and their
    // exclusive snapshot payload types are gone.
    assert!(!query_field_names.contains(&"discoverySyncStatus"));
    assert!(!query_field_names.contains(&"libraryScanSession"));
    assert!(!query_field_names.contains(&"mediaServerConnection"));
    assert!(!query_field_names.contains(&"outboundRateLimitSnapshot"));
    assert!(!query_field_names.contains(&"upstreamSchedulerSnapshot"));
    assert!(mutation_field_names.contains(&"queueReplacementRelease"));
    assert!(!public_type_names.contains(&"OutboundRateLimitSnapshotPayload"));
    assert!(!public_type_names.contains(&"OutboundHostRpsSnapshotEntryPayload"));
    assert!(!public_type_names.contains(&"OutboundDestinationCooldownSnapshotEntryPayload"));
    assert!(!public_type_names.contains(&"UpstreamSchedulerSnapshotPayload"));
    assert!(!public_type_names.contains(&"UpstreamSchedulerSnapshotEntryPayload"));

    // 0.17.0 API surface trim (field wave): never-selected reachable types are
    // gone. FinalizeExternalImportPayload.librarySettingApplications was the only anchor
    // for the external-import library-setting projection, so the trio payload types and
    // their exclusive enums drop with the field; DiscoverySyncRunPayload was only reachable
    // through the removed DiscoverySyncStatusPayload.recentRuns field.
    assert!(!public_type_names.contains(&"DiscoverySyncRunPayload"));
    assert!(!public_type_names.contains(&"ExternalImportLibrarySettingApplicationPayload"));
    assert!(!public_type_names.contains(&"ExternalImportLibrarySettingEvidencePayload"));
    assert!(!public_type_names.contains(&"ExternalImportLibrarySettingValuePayload"));
    assert!(!public_type_names.contains(&"ExternalImportLibrarySettingKey"));
    assert!(!public_type_names.contains(&"ExternalImportLibrarySettingConfidence"));
    assert!(!public_type_names.contains(&"ExternalImportLibrarySettingDisposition"));
}

/// US3/FR-076: a client may ask for a managed move or for adoption, and may not
/// ask for the catalog-only fast path. That is a schema-level guarantee, not a
/// resolver check, so it is asserted against the published input enum.
#[tokio::test]
async fn graphql_introspection_location_mode_is_requestable_but_never_catalog_only() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          modeInput: __type(name: "LocationExecutionModeInput") {
            kind
            enumValues { name }
          }
          modeValue: __type(name: "LocationExecutionModeValue") {
            enumValues { name }
          }
          previewInput: __type(name: "LocationOperationPreviewInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          startInput: __type(name: "StartLocationOperationInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    assert_eq!(body["data"]["modeInput"]["kind"], "ENUM");
    let requestable = body["data"]["modeInput"]["enumValues"]
        .as_array()
        .expect("LocationExecutionModeInput should expose enumValues")
        .iter()
        .map(|value| value["name"].as_str().expect("enum value name"))
        .collect::<Vec<_>>();
    assert_eq!(
        requestable,
        vec!["MOVE_WITH_SCRYER", "FILES_ALREADY_THERE"],
        "only the two modes a caller may ask for: {body}"
    );

    // The reported enum keeps the derived value the input enum refuses.
    let reported = body["data"]["modeValue"]["enumValues"]
        .as_array()
        .expect("LocationExecutionModeValue should expose enumValues")
        .iter()
        .map(|value| value["name"].as_str().expect("enum value name"))
        .collect::<Vec<_>>();
    assert!(
        reported.contains(&"CATALOG_ONLY"),
        "the fileless fast path is still reported: {body}"
    );

    // Optional on both inputs: an existing client that never sends the field
    // keeps asking for the managed move it always asked for.
    for input_alias in ["previewInput", "startInput"] {
        let mode = body["data"][input_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{input_alias} should expose inputFields"))
            .iter()
            .find(|field| field["name"] == "mode")
            .unwrap_or_else(|| panic!("{input_alias}.mode should exist: {body}"));
        assert_eq!(mode["type"]["kind"], "ENUM", "{input_alias}.mode");
        assert_eq!(
            mode["type"]["name"], "LocationExecutionModeInput",
            "{input_alias}.mode"
        );
    }
}

#[tokio::test]
async fn graphql_introspection_external_import_finalize_settings_payload_is_trimmed() {
    // 0.17.0 API surface trim (field wave): the never-selected library-setting
    // projection on FinalizeExternalImportPayload was removed, so the trio payload types
    // and their exclusive enums no longer appear in the schema.
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          finalizePayload: __type(name: "FinalizeExternalImportPayload") {
            fields { name }
          }
          applicationPayload: __type(name: "ExternalImportLibrarySettingApplicationPayload") {
            name
          }
          valuePayload: __type(name: "ExternalImportLibrarySettingValuePayload") {
            name
          }
          settingKey: __type(name: "ExternalImportLibrarySettingKey") {
            name
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let finalize_fields: Vec<&str> = body["data"]["finalizePayload"]["fields"]
        .as_array()
        .expect("fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(finalize_fields, vec!["monitorWarmupSessionId"]);

    assert!(body["data"]["applicationPayload"].is_null());
    assert!(body["data"]["valuePayload"].is_null());
    assert!(body["data"]["settingKey"].is_null());
}

#[tokio::test]
async fn graphql_introspection_plugin_mutations_use_progress_api() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields {
              name
              args { name type { kind name ofType { kind name } } }
            }
          }
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          installPluginInput: __type(name: "InstallPluginInput") { name }
          uninstallPluginInput: __type(name: "UninstallPluginInput") { name }
          upgradePluginInput: __type(name: "UpgradePluginInput") { name }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    let fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let names: Vec<&str> = fields.iter().filter_map(|f| f["name"].as_str()).collect();

    assert!(names.contains(&"refreshPluginCatalog"));
    assert!(names.contains(&"beginInstallPlugin"));
    assert!(names.contains(&"beginUpgradePlugin"));
    assert!(names.contains(&"installManualPlugin"));
    assert!(names.contains(&"installUploadedPlugin"));
    assert!(!names.contains(&"refreshPluginRegistry"));
    assert!(!names.contains(&"installPlugin"));
    assert!(!names.contains(&"upgradePlugin"));
    assert!(body["data"]["installPluginInput"].is_null());
    assert!(body["data"]["uninstallPluginInput"].is_null());
    assert!(body["data"]["upgradePluginInput"].is_null());

    let plugin_id_arg = |field_name: &str| {
        fields
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{field_name} should exist"))["args"]
            .as_array()
            .expect("plugin mutation should expose args")
            .iter()
            .find(|arg| arg["name"] == "pluginId")
            .unwrap_or_else(|| panic!("{field_name}.pluginId should exist"))
            .clone()
    };
    for field_name in [
        "beginInstallPlugin",
        "uninstallPlugin",
        "beginUpgradePlugin",
    ] {
        let arg = plugin_id_arg(field_name);
        assert_eq!(arg["type"]["kind"], "NON_NULL", "{field_name}");
        assert_eq!(arg["type"]["ofType"]["name"], "ID", "{field_name}");
    }
}

#[tokio::test]
async fn graphql_introspection_media_request_actions_use_direct_request_id() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields {
              name
              args { name type { kind name ofType { kind name } } }
            }
          }
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          mediaRequestActionInput: __type(name: "MediaRequestActionInput") { name }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["mediaRequestActionInput"].is_null());

    let fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let request_id_arg = |field_name: &str| {
        fields
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{field_name} should exist"))["args"]
            .as_array()
            .expect("media request mutation should expose args")
            .iter()
            .find(|arg| arg["name"] == "requestId")
            .unwrap_or_else(|| panic!("{field_name}.requestId should exist"))
            .clone()
    };
    for field_name in ["dismissMediaRequest", "cancelMyMediaRequest"] {
        let arg = request_id_arg(field_name);
        assert_eq!(arg["type"]["kind"], "NON_NULL", "{field_name}");
        assert_eq!(arg["type"]["ofType"]["name"], "ID", "{field_name}");
    }
}

#[tokio::test]
async fn graphql_introspection_media_request_inputs_use_id_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          submitInput: __type(name: "SubmitMediaRequestInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          approveInput: __type(name: "ApproveMediaRequestInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          updateInput: __type(name: "UpdateMediaRequestInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let field = |type_alias: &str, name: &str| {
        body["data"][type_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose inputFields"))
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("{type_alias}.{name} should exist"))
            .clone()
    };
    for (type_alias, name) in [
        ("submitInput", "libraryId"),
        ("approveInput", "requestId"),
        ("approveInput", "qualityProfileId"),
        ("updateInput", "requestId"),
        ("updateInput", "requestedQualityProfileId"),
    ] {
        let field = field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{type_alias}.{name}");
        assert_eq!(field["type"]["ofType"]["name"], "ID", "{type_alias}.{name}");
    }
    let requested_quality_profile_id = field("submitInput", "requestedQualityProfileId");
    assert_eq!(
        requested_quality_profile_id["type"]["kind"], "SCALAR",
        "submitInput.requestedQualityProfileId"
    );
    assert_eq!(
        requested_quality_profile_id["type"]["name"], "ID",
        "submitInput.requestedQualityProfileId"
    );
}

#[tokio::test]
async fn graphql_introspection_media_requests_changed_uses_typed_payload() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          subscriptionRoot: __type(name: "SubscriptionRoot") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          payload: __type(name: "MediaRequestChangedPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let subscription_fields = body["data"]["subscriptionRoot"]["fields"]
        .as_array()
        .expect("SubscriptionRoot should expose fields");
    let media_requests_changed = subscription_fields
        .iter()
        .find(|field| field["name"] == "mediaRequestsChanged")
        .expect("mediaRequestsChanged should exist");
    assert_eq!(media_requests_changed["type"]["kind"], "NON_NULL");
    assert_eq!(
        media_requests_changed["type"]["ofType"]["name"],
        "MediaRequestChangedPayload"
    );

    let payload_fields = body["data"]["payload"]["fields"]
        .as_array()
        .expect("MediaRequestChangedPayload should expose fields");
    let field = |name: &str| {
        payload_fields
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("MediaRequestChangedPayload.{name} should exist"))
    };
    for name in ["eventId", "requestId", "libraryId"] {
        let field = field(name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{name}");
        assert_eq!(field["type"]["ofType"]["name"], "ID", "{name}");
    }
    // 0.17.0 trim: the denormalized created/requested/approved id+name fields
    // were removed from MediaRequestChangedPayload (never selected by clients).
    let event_type = field("eventType");
    assert_eq!(event_type["type"]["kind"], "NON_NULL");
    assert_eq!(event_type["type"]["ofType"]["name"], "DomainEventTypeValue");
}

#[tokio::test]
async fn graphql_introspection_provider_catalog_changed_uses_family_enum() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          subscriptionRoot: __type(name: "SubscriptionRoot") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let subscription_fields = body["data"]["subscriptionRoot"]["fields"]
        .as_array()
        .expect("SubscriptionRoot should expose fields");
    let provider_catalog_changed = subscription_fields
        .iter()
        .find(|field| field["name"] == "providerCatalogChanged")
        .expect("providerCatalogChanged should exist");
    assert_eq!(provider_catalog_changed["type"]["kind"], "NON_NULL");
    assert_eq!(provider_catalog_changed["type"]["ofType"]["kind"], "LIST");
    assert_eq!(
        provider_catalog_changed["type"]["ofType"]["ofType"]["kind"],
        "NON_NULL"
    );
    assert_eq!(
        provider_catalog_changed["type"]["ofType"]["ofType"]["ofType"]["name"],
        "ProviderCatalogFamilyValue"
    );
}

#[tokio::test]
async fn graphql_introspection_plugin_install_progress_uses_plugin_id() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          subscriptionRoot: __type(name: "SubscriptionRoot") {
            fields {
              name
              args { name type { kind name ofType { kind name } } }
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let subscription_fields = body["data"]["subscriptionRoot"]["fields"]
        .as_array()
        .expect("SubscriptionRoot should expose fields");
    let plugin_install_progress = subscription_fields
        .iter()
        .find(|field| field["name"] == "pluginInstallProgress")
        .expect("pluginInstallProgress should exist");
    let plugin_id = plugin_install_progress["args"]
        .as_array()
        .expect("pluginInstallProgress should expose args")
        .iter()
        .find(|arg| arg["name"] == "pluginId")
        .expect("pluginId should exist");
    assert_eq!(plugin_id["type"]["kind"], "NON_NULL");
    assert_eq!(plugin_id["type"]["ofType"]["name"], "ID");
}

#[tokio::test]
async fn graphql_introspection_delete_previews_use_direct_ids() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
            }
          }
          deleteTitlePreviewInput: __type(name: "DeleteTitlePreviewInput") { name }
          deleteMediaFilePreviewInput: __type(name: "DeleteMediaFilePreviewInput") { name }
          deleteExternalSubtitlePreviewInput: __type(name: "DeleteExternalSubtitlePreviewInput") { name }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);
    assert!(body["data"]["deleteTitlePreviewInput"].is_null());
    assert!(body["data"]["deleteMediaFilePreviewInput"].is_null());
    assert!(body["data"]["deleteExternalSubtitlePreviewInput"].is_null());

    let fields = body["data"]["queryRoot"]["fields"]
        .as_array()
        .expect("QueryRoot should expose fields");
    let id_arg = |field_name: &str, arg_name: &str| {
        fields
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{field_name} should exist"))["args"]
            .as_array()
            .expect("preview query should expose args")
            .iter()
            .find(|arg| arg["name"] == arg_name)
            .unwrap_or_else(|| panic!("{field_name}.{arg_name} should exist"))
            .clone()
    };
    for (field_name, arg_name) in [
        ("deleteTitlePreview", "titleId"),
        ("deleteMediaFilePreview", "fileId"),
        ("deleteExternalSubtitlePreview", "externalSubtitleId"),
    ] {
        let arg = id_arg(field_name, arg_name);
        assert_eq!(arg["type"]["kind"], "NON_NULL", "{field_name}");
        assert_eq!(arg["type"]["ofType"]["name"], "ID", "{field_name}");
    }
}

#[tokio::test]
async fn graphql_introspection_uninstall_plugin_uses_payload_result() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
	              type {
	                kind
	                name
	                ofType {
	                  kind
	                  name
	                }
	              }
	            }
          }
          uninstallPayload: __type(name: "UninstallPluginPayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let uninstall = mutation_fields
        .iter()
        .find(|field| field["name"] == "uninstallPlugin")
        .expect("uninstallPlugin should exist");
    assert_eq!(
        uninstall["type"]["ofType"]["name"],
        "UninstallPluginPayload"
    );

    let payload_fields: Vec<&str> = body["data"]["uninstallPayload"]["fields"]
        .as_array()
        .expect("UninstallPluginPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(payload_fields, vec!["pluginId"]);
}

#[tokio::test]
async fn graphql_introspection_query_root_uses_semantic_search_and_browse_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"{ __type(name: "QueryRoot") { fields { name } } }"#,
        json!({}),
    )
    .await;
    let fields = body["data"]["__type"]["fields"]
        .as_array()
        .expect("should have fields");
    let names: Vec<&str> = fields.iter().filter_map(|f| f["name"].as_str()).collect();

    assert!(names.contains(&"searchReleases"));
    assert!(!names.contains(&"searchIndexers"));
    assert!(!names.contains(&"searchIndexersEpisode"));
    assert!(!names.contains(&"searchIndexersForTitle"));
    assert!(!names.contains(&"searchIndexersForEpisode"));
    assert!(!names.contains(&"titleCollections"));
    assert!(!names.contains(&"collectionEpisodes"));
    assert!(!names.contains(&"titleMediaFiles"));
    assert!(names.contains(&"wantedItem"));
    assert!(!names.contains(&"pendingRelease"));
    assert!(names.contains(&"titleHistory"));
    assert!(!names.contains(&"titleEvents"));
    assert!(!names.contains(&"episodeHistory"));
    assert!(!names.contains(&"libraryScanSession"));
    assert!(!names.contains(&"domainEvents"));
    assert!(names.contains(&"downloadHistory"));
}

#[tokio::test]
async fn graphql_introspection_pending_releases_uses_page_payload() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          page: __type(name: "PendingReleasesPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          filter: __type(name: "PendingReleaseFilterInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let query_fields = body["data"]["queryRoot"]["fields"]
        .as_array()
        .expect("QueryRoot should expose fields");
    let pending_releases = query_fields
        .iter()
        .find(|field| field["name"] == "pendingReleases")
        .expect("pendingReleases should exist");
    assert_eq!(pending_releases["type"]["kind"], "NON_NULL");
    assert_eq!(
        pending_releases["type"]["ofType"]["name"],
        "PendingReleasesPayload"
    );

    let pending_arg = |name: &str| {
        pending_releases["args"]
            .as_array()
            .expect("pendingReleases should expose args")
            .iter()
            .find(|arg| arg["name"] == name)
            .unwrap_or_else(|| panic!("pendingReleases.{name} arg should exist"))
            .clone()
    };
    let filter_arg = pending_arg("filter");
    assert_eq!(filter_arg["type"]["kind"], "INPUT_OBJECT");
    assert_eq!(filter_arg["type"]["name"], "PendingReleaseFilterInput");
    for arg_name in ["limit", "offset"] {
        let arg = pending_arg(arg_name);
        assert_eq!(arg["type"]["kind"], "NON_NULL", "{arg_name}");
        assert_eq!(arg["type"]["ofType"]["name"], "Int", "{arg_name}");
    }

    let page_fields = body["data"]["page"]["fields"]
        .as_array()
        .expect("PendingReleasesPayload should expose fields");
    let page_field = |name: &str| {
        page_fields
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("PendingReleasesPayload.{name} should exist"))
    };
    assert_eq!(page_field("items")["type"]["kind"], "NON_NULL");
    assert_eq!(page_field("hasMore")["type"]["ofType"]["name"], "Boolean");
    assert_eq!(page_field("totalCount")["type"]["ofType"]["name"], "Int");

    let filter_fields = body["data"]["filter"]["inputFields"]
        .as_array()
        .expect("PendingReleaseFilterInput should expose fields");
    let filter_field = |name: &str| {
        filter_fields
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("PendingReleaseFilterInput.{name} should exist"))
    };
    for name in ["titleId", "wantedItemId"] {
        assert_eq!(filter_field(name)["type"]["kind"], "SCALAR", "{name}");
        assert_eq!(filter_field(name)["type"]["name"], "ID", "{name}");
    }
    let statuses = filter_field("statuses");
    assert_eq!(statuses["type"]["kind"], "LIST");
    assert_eq!(statuses["type"]["ofType"]["kind"], "NON_NULL");
    assert_eq!(
        statuses["type"]["ofType"]["ofType"]["name"],
        "PendingReleaseStatusValue"
    );
}

#[tokio::test]
async fn graphql_introspection_search_metadata_uses_media_facet_enum() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields {
              name
	              args {
	                name
	                type {
	                  kind
	                  name
	                  ofType {
	                    kind
	                    name
	                    ofType {
	                      kind
	                      name
	                    }
	                  }
	                }
	              }
            }
          }
          metadataMovieInput: __type(name: "MetadataMovieInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          metadataSeriesInput: __type(name: "MetadataSeriesInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["queryRoot"]["fields"]
        .as_array()
        .expect("QueryRoot should expose fields");
    let search_metadata = fields
        .iter()
        .find(|field| field["name"] == "searchMetadata")
        .expect("searchMetadata should exist");
    let args = search_metadata["args"]
        .as_array()
        .expect("searchMetadata should expose args");
    let type_arg = args
        .iter()
        .find(|arg| arg["name"] == "type")
        .expect("searchMetadata.type should exist");
    assert_eq!(type_arg["type"]["kind"], "NON_NULL");
    assert_eq!(type_arg["type"]["ofType"]["name"], "MediaFacetValue");

    for (field_name, input_name) in [
        ("metadataMovie", "MetadataMovieInput"),
        ("metadataSeries", "MetadataSeriesInput"),
    ] {
        let field = fields
            .iter()
            .find(|field| field["name"] == field_name)
            .expect("metadata lookup field should exist");
        let args = field["args"]
            .as_array()
            .expect("metadata lookup should expose args");
        assert_eq!(args.len(), 1);
        assert_eq!(args[0]["name"], "input");
        assert_eq!(args[0]["type"]["kind"], "NON_NULL");
        assert_eq!(args[0]["type"]["ofType"]["name"], input_name);
    }

    let movie_input_fields = body["data"]["metadataMovieInput"]["inputFields"]
        .as_array()
        .expect("MetadataMovieInput should expose fields");
    assert_eq!(
        movie_input_fields
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect::<Vec<_>>(),
        vec!["tvdbId", "smgId", "tmdbId", "imdbId", "language"]
    );
    for (field, scalar) in [
        (&movie_input_fields[0], "String"),
        (&movie_input_fields[1], "Int"),
        (&movie_input_fields[2], "Int"),
        (&movie_input_fields[3], "String"),
    ] {
        assert_eq!(field["type"]["kind"], "SCALAR");
        assert_eq!(field["type"]["name"], scalar);
    }

    let series_input_fields = body["data"]["metadataSeriesInput"]["inputFields"]
        .as_array()
        .expect("MetadataSeriesInput should expose fields");
    assert_eq!(
        series_input_fields
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect::<Vec<_>>(),
        vec!["tvdbId", "includeEpisodes", "language"]
    );
    assert_eq!(series_input_fields[0]["type"]["kind"], "NON_NULL");
    assert_eq!(series_input_fields[0]["type"]["ofType"]["name"], "String");
}

#[tokio::test]
async fn graphql_introspection_begin_manual_import_selection_uses_input_object() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
	          mutationRoot: __type(name: "MutationRoot") {
	            fields {
	              name
	              args {
	                name
	                type {
	                  kind
	                  name
	                  ofType {
	                    kind
	                    name
	                    ofType {
	                      kind
	                      name
	                    }
	                  }
	                }
	              }
	            }
	          }
          beginManualImportSelectionInput: __type(name: "BeginManualImportSelectionInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          manualImportFilePreview: __type(name: "ManualImportFilePreviewPayload") {
            fields { name type { kind name } }
          }
          manualImportVideoFacts: __type(name: "ManualImportVideoFactsPayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let begin_manual_import_selection = fields
        .iter()
        .find(|field| field["name"] == "beginManualImportSelection")
        .expect("beginManualImportSelection should exist");
    let args = begin_manual_import_selection["args"]
        .as_array()
        .expect("beginManualImportSelection should expose args");
    assert_eq!(args.len(), 1);
    assert_eq!(args[0]["name"], "input");
    assert_eq!(args[0]["type"]["kind"], "NON_NULL");
    assert_eq!(
        args[0]["type"]["ofType"]["name"],
        "BeginManualImportSelectionInput"
    );

    let input_fields = body["data"]["beginManualImportSelectionInput"]["inputFields"]
        .as_array()
        .expect("BeginManualImportSelectionInput should expose input fields");
    let input_field = |name: &str| {
        input_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("input field should exist")
    };
    let client_id = input_field("clientId");
    assert_eq!(client_id["type"]["kind"], "NON_NULL");
    assert_eq!(client_id["type"]["ofType"]["name"], "ID");

    let download_client_item_id = input_field("downloadClientItemId");
    assert_eq!(download_client_item_id["type"]["kind"], "NON_NULL");
    assert_eq!(download_client_item_id["type"]["ofType"]["name"], "String");

    let title_id = input_field("titleId");
    assert_eq!(title_id["type"]["kind"], "NON_NULL");
    assert_eq!(title_id["type"]["ofType"]["name"], "ID");

    let preview_fields = body["data"]["manualImportFilePreview"]["fields"]
        .as_array()
        .expect("ManualImportFilePreviewPayload should expose fields");
    let video_facts = preview_fields
        .iter()
        .find(|field| field["name"] == "videoFacts")
        .expect("manual import preview should expose video facts");
    assert_eq!(video_facts["type"]["kind"], "OBJECT");
    assert_eq!(video_facts["type"]["name"], "ManualImportVideoFactsPayload");

    let fact_field_names = body["data"]["manualImportVideoFacts"]["fields"]
        .as_array()
        .expect("ManualImportVideoFactsPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        fact_field_names,
        vec![
            "containerFormat",
            "videoCodec",
            "audioCodec",
            "videoWidth",
            "videoHeight",
            "durationSeconds",
        ]
    );

    let extract_archives = input_field("extractArchives");
    assert_eq!(extract_archives["type"]["kind"], "SCALAR");
    assert_eq!(extract_archives["type"]["name"], "Boolean");
}

#[tokio::test]
async fn graphql_introspection_title_history_filter_uses_event_type_enum() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          filterInput: __type(name: "TitleHistoryFilterInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
            }
          }
          eventType: __type(name: "TitleHistoryEventTypeValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["filterInput"]["inputFields"]
        .as_array()
        .expect("TitleHistoryFilterInput should expose input fields");
    let event_types = fields
        .iter()
        .find(|field| field["name"] == "eventTypes")
        .expect("eventTypes input should exist");
    assert_eq!(event_types["type"]["kind"], "LIST");
    assert_eq!(
        event_types["type"]["ofType"]["ofType"]["name"],
        "TitleHistoryEventTypeValue"
    );
    let id_list_input = |name: &str| {
        fields
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("{name} input should exist"))
    };
    for name in ["titleIds", "libraryIds"] {
        let field = id_list_input(name);
        assert_eq!(field["type"]["kind"], "LIST", "{name}");
        assert_eq!(field["type"]["ofType"]["kind"], "NON_NULL", "{name}");
        assert_eq!(field["type"]["ofType"]["ofType"]["name"], "ID", "{name}");
    }
    let episode_id = id_list_input("episodeId");
    assert_eq!(episode_id["type"]["kind"], "SCALAR");
    assert_eq!(episode_id["type"]["name"], "ID");

    let names: Vec<&str> = body["data"]["eventType"]["enumValues"]
        .as_array()
        .expect("TitleHistoryEventTypeValue should expose enum values")
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert!(names.contains(&"DOWNLOAD_FAILED"));
    assert!(names.contains(&"DOWNLOAD_IGNORED"));
    assert!(names.contains(&"REMATCHED"));
}

#[tokio::test]
async fn graphql_introspection_recycle_bin_uses_id_and_payload_results() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          recycledItem: __type(name: "RecycledItemPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          restorePayload: __type(name: "RestoreRecycledItemPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          restoreBatchPayload: __type(name: "RestoreRecycledItemsPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deletePayload: __type(name: "DeleteRecycledItemPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deleteBatchPayload: __type(name: "DeleteRecycledItemsPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          emptyPayload: __type(name: "EmptyRecycleBinPayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let recycled_fields = body["data"]["recycledItem"]["fields"]
        .as_array()
        .expect("RecycledItemPayload should expose fields");
    let recycled_field = |name: &str| {
        recycled_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("recycled item field should exist")
    };
    assert_eq!(recycled_field("id")["type"]["kind"], "NON_NULL");
    assert_eq!(recycled_field("id")["type"]["ofType"]["name"], "ID");
    assert_eq!(recycled_field("libraryId")["type"]["kind"], "NON_NULL");
    assert_eq!(recycled_field("libraryId")["type"]["ofType"]["name"], "ID");
    assert_eq!(recycled_field("titleId")["type"]["name"], "ID");
    assert_eq!(recycled_field("titleName")["type"]["name"], "String");

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation = |name: &str| {
        mutation_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("mutation field should exist")
    };
    let restore = mutation("restoreRecycledItem");
    assert_eq!(
        restore["type"]["ofType"]["name"],
        "RestoreRecycledItemPayload"
    );
    let restore_id_arg = restore["args"]
        .as_array()
        .unwrap()
        .iter()
        .find(|arg| arg["name"] == "id")
        .expect("restore id arg should exist");
    assert_eq!(restore_id_arg["type"]["kind"], "NON_NULL");
    assert_eq!(restore_id_arg["type"]["ofType"]["name"], "ID");

    let restore_batch = mutation("restoreRecycledItems");
    assert_eq!(
        restore_batch["type"]["ofType"]["name"],
        "RestoreRecycledItemsPayload"
    );
    let restore_batch_input = restore_batch["args"]
        .as_array()
        .unwrap()
        .iter()
        .find(|arg| arg["name"] == "input")
        .expect("batch restore input should exist");
    assert_eq!(restore_batch_input["type"]["kind"], "NON_NULL");
    assert_eq!(
        restore_batch_input["type"]["ofType"]["name"],
        "RestoreRecycledItemsInput"
    );

    let delete = mutation("deleteRecycledItem");
    assert_eq!(
        delete["type"]["ofType"]["name"],
        "DeleteRecycledItemPayload"
    );
    let delete_batch = mutation("deleteRecycledItems");
    assert_eq!(
        delete_batch["type"]["ofType"]["name"],
        "DeleteRecycledItemsPayload"
    );
    let delete_batch_input = delete_batch["args"]
        .as_array()
        .unwrap()
        .iter()
        .find(|arg| arg["name"] == "input")
        .expect("batch delete input should exist");
    assert_eq!(delete_batch_input["type"]["kind"], "NON_NULL");
    assert_eq!(
        delete_batch_input["type"]["ofType"]["name"],
        "DeleteRecycledItemsInput"
    );
    let empty = mutation("emptyRecycleBin");
    assert_eq!(empty["type"]["ofType"]["name"], "EmptyRecycleBinPayload");

    let restore_payload_fields = body["data"]["restorePayload"]["fields"]
        .as_array()
        .expect("RestoreRecycledItemPayload should expose fields");
    let restore_id = restore_payload_fields
        .iter()
        .find(|field| field["name"] == "id")
        .expect("restore payload id field should exist");
    assert_eq!(restore_id["type"]["ofType"]["name"], "ID");
    let restore_job_run = restore_payload_fields
        .iter()
        .find(|field| field["name"] == "jobRun")
        .expect("restore payload job run field should exist");
    assert_eq!(restore_job_run["type"]["kind"], "NON_NULL");
    assert_eq!(restore_job_run["type"]["ofType"]["name"], "JobRunPayload");

    for (payload_name, payload) in [
        (
            "RestoreRecycledItemsPayload",
            &body["data"]["restoreBatchPayload"],
        ),
        (
            "DeleteRecycledItemsPayload",
            &body["data"]["deleteBatchPayload"],
        ),
    ] {
        let fields = payload["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{payload_name} should expose fields"));
        let ids = fields
            .iter()
            .find(|field| field["name"] == "ids")
            .expect("batch payload ids should exist");
        assert_eq!(ids["type"]["kind"], "NON_NULL");
        assert_eq!(ids["type"]["ofType"]["kind"], "LIST");
        let job_run = fields
            .iter()
            .find(|field| field["name"] == "jobRun")
            .expect("batch payload job run should exist");
        assert_eq!(job_run["type"]["kind"], "NON_NULL");
        assert_eq!(job_run["type"]["ofType"]["name"], "JobRunPayload");
    }

    let delete_payload_fields = body["data"]["deletePayload"]["fields"]
        .as_array()
        .expect("DeleteRecycledItemPayload should expose fields");
    let delete_id = delete_payload_fields
        .iter()
        .find(|field| field["name"] == "id")
        .expect("delete payload id field should exist");
    assert_eq!(delete_id["type"]["ofType"]["name"], "ID");

    let empty_payload_names: Vec<&str> = body["data"]["emptyPayload"]["fields"]
        .as_array()
        .expect("EmptyRecycleBinPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(empty_payload_names, vec!["purgedCount"]);
}

#[tokio::test]
async fn graphql_introspection_notification_mutations_use_id_and_payload_results() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deleteChannelPayload: __type(name: "DeleteNotificationChannelPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          testChannelPayload: __type(name: "NotificationChannelTestPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deleteSubscriptionPayload: __type(name: "DeleteNotificationSubscriptionPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          channelPayload: __type(name: "NotificationChannelPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          subscriptionPayload: __type(name: "NotificationSubscriptionPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          targetPayload: __type(name: "NotificationTargetPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          createChannelInput: __type(name: "CreateNotificationChannelInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          updateChannelInput: __type(name: "UpdateNotificationChannelInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          createSubscriptionInput: __type(name: "CreateNotificationSubscriptionInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          updateSubscriptionInput: __type(name: "UpdateNotificationSubscriptionInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation = |name: &str| {
        mutation_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("mutation field should exist")
    };

    fn id_arg(field: &Value) -> &Value {
        field["args"]
            .as_array()
            .expect("mutation should expose args")
            .iter()
            .find(|arg| arg["name"] == "id")
            .expect("id arg should exist")
    }

    let delete_channel = mutation("deleteNotificationChannel");
    assert_eq!(
        delete_channel["type"]["ofType"]["name"],
        "DeleteNotificationChannelPayload"
    );
    assert_eq!(id_arg(delete_channel)["type"]["kind"], "NON_NULL");
    assert_eq!(id_arg(delete_channel)["type"]["ofType"]["name"], "ID");

    let test_channel = mutation("testNotificationChannel");
    assert_eq!(
        test_channel["type"]["ofType"]["name"],
        "NotificationChannelTestPayload"
    );
    assert_eq!(id_arg(test_channel)["type"]["kind"], "NON_NULL");
    assert_eq!(id_arg(test_channel)["type"]["ofType"]["name"], "ID");

    let delete_subscription = mutation("deleteNotificationSubscription");
    assert_eq!(
        delete_subscription["type"]["ofType"]["name"],
        "DeleteNotificationSubscriptionPayload"
    );
    assert_eq!(id_arg(delete_subscription)["type"]["kind"], "NON_NULL");
    assert_eq!(id_arg(delete_subscription)["type"]["ofType"]["name"], "ID");

    let delete_channel_fields = body["data"]["deleteChannelPayload"]["fields"]
        .as_array()
        .expect("DeleteNotificationChannelPayload should expose fields");
    let delete_channel_id = delete_channel_fields
        .iter()
        .find(|field| field["name"] == "id")
        .expect("delete channel payload id field should exist");
    assert_eq!(delete_channel_id["type"]["ofType"]["name"], "ID");

    let test_channel_names: Vec<&str> = body["data"]["testChannelPayload"]["fields"]
        .as_array()
        .expect("NotificationChannelTestPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(
        test_channel_names,
        vec!["id", "status", "message", "retryAfterSeconds"]
    );

    let delete_subscription_fields = body["data"]["deleteSubscriptionPayload"]["fields"]
        .as_array()
        .expect("DeleteNotificationSubscriptionPayload should expose fields");
    let delete_subscription_id = delete_subscription_fields
        .iter()
        .find(|field| field["name"] == "id")
        .expect("delete subscription payload id field should exist");
    assert_eq!(delete_subscription_id["type"]["ofType"]["name"], "ID");

    let output_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    let input_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose input fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    let assert_non_null_id = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["name"], "ID", "{label}");
    };
    let assert_optional_id = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "SCALAR", "{label}");
        assert_eq!(field["type"]["name"], "ID", "{label}");
    };
    for (type_alias, field_name) in [
        ("channelPayload", "id"),
        ("subscriptionPayload", "id"),
        ("subscriptionPayload", "targetId"),
        ("targetPayload", "id"),
    ] {
        assert_non_null_id(
            output_field(type_alias, field_name),
            &format!("{type_alias}.{field_name}"),
        );
    }
    for (type_alias, field_name) in [
        ("channelPayload", "mediaServerConnectionId"),
        ("subscriptionPayload", "channelId"),
        ("targetPayload", "mediaServerConnectionId"),
    ] {
        assert_optional_id(
            output_field(type_alias, field_name),
            &format!("{type_alias}.{field_name}"),
        );
    }
    for (type_alias, field_name) in [
        ("updateChannelInput", "id"),
        ("updateSubscriptionInput", "id"),
    ] {
        assert_non_null_id(
            input_field(type_alias, field_name),
            &format!("{type_alias}.{field_name}"),
        );
    }
    for (type_alias, field_name) in [
        ("createChannelInput", "mediaServerConnectionId"),
        ("updateChannelInput", "mediaServerConnectionId"),
        ("createSubscriptionInput", "channelId"),
        ("createSubscriptionInput", "targetId"),
        ("updateSubscriptionInput", "targetId"),
    ] {
        assert_optional_id(
            input_field(type_alias, field_name),
            &format!("{type_alias}.{field_name}"),
        );
    }
}

#[tokio::test]
async fn graphql_introspection_provider_tests_use_validation_payload() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          providerValidation: __type(name: "ProviderValidationPayload") {
            fields { name }
          }
          testMediaServerConnectionInput: __type(name: "TestMediaServerConnectionInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation = |name: &str| {
        mutation_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("mutation field should exist")
    };

    for name in [
        "testIndexerConnection",
        "testDownloadClientConnection",
        "testSubtitleProviderConnection",
        "testMediaServerConnection",
    ] {
        assert_eq!(
            mutation(name)["type"]["ofType"]["name"],
            "ProviderValidationPayload",
            "{name} should return ProviderValidationPayload"
        );
    }

    let media_server_test = mutation("testMediaServerConnection");
    let media_server_args = media_server_test["args"]
        .as_array()
        .expect("testMediaServerConnection should expose args");
    assert_eq!(media_server_args.len(), 1);
    assert_eq!(media_server_args[0]["name"], "input");
    assert_eq!(media_server_args[0]["type"]["kind"], "NON_NULL");
    assert_eq!(
        media_server_args[0]["type"]["ofType"]["name"],
        "TestMediaServerConnectionInput"
    );

    let media_server_input_fields = body["data"]["testMediaServerConnectionInput"]["inputFields"]
        .as_array()
        .expect("TestMediaServerConnectionInput should expose input fields");
    let id_field = media_server_input_fields
        .iter()
        .find(|field| field["name"] == "id")
        .expect("id field should exist");
    assert_eq!(id_field["type"]["kind"], "NON_NULL");
    assert_eq!(id_field["type"]["ofType"]["name"], "ID");
    let plex_auth_token_field = media_server_input_fields
        .iter()
        .find(|field| field["name"] == "plexAuthToken")
        .expect("plexAuthToken field should exist");
    assert_eq!(plex_auth_token_field["type"]["name"], "String");

    let payload_fields: Vec<&str> = body["data"]["providerValidation"]["fields"]
        .as_array()
        .expect("ProviderValidationPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(
        payload_fields,
        vec!["status", "message", "retryAfterSeconds"]
    );
}

#[tokio::test]
async fn graphql_introspection_provider_configs_use_typed_config_values() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          configValuePayload: __type(name: "ProviderConfigValuePayload") {
            fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          configFieldValue: __type(name: "ProviderConfigFieldValue") {
            possibleTypes { name }
          }
          configValueInput: __type(name: "ProviderConfigValueInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          indexerPayload: __type(name: "IndexerConfigPayload") {
            fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          downloadClientPayload: __type(name: "DownloadClientConfigPayload") {
            fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          indexerSyncPayload: __type(name: "IndexerConfigSyncPayload") {
            fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          notificationChannelPayload: __type(name: "NotificationChannelPayload") {
            fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          createIndexerInput: __type(name: "CreateIndexerConfigInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          updateIndexerInput: __type(name: "UpdateIndexerConfigInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          indexerMappingCatalog: __type(name: "IndexerDownloadClientMappingCatalogPayload") {
            fields { name }
          }
          indexerProviderCompatibility: __type(name: "IndexerDownloadClientProviderCompatibilityPayload") {
            fields { name }
          }
          testIndexerInput: __type(name: "TestIndexerConnectionInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          createDownloadClientInput: __type(name: "CreateDownloadClientConfigInput") {
            inputFields { name }
          }
          updateDownloadClientInput: __type(name: "UpdateDownloadClientConfigInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          reorderDownloadClientInput: __type(name: "ReorderDownloadClientConfigsInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          reorderDownloadClientPayload: __type(name: "ReorderDownloadClientConfigsPayload") {
            fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          testDownloadClientInput: __type(name: "TestDownloadClientConnectionInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          createSubtitleProviderInput: __type(name: "CreateSubtitleProviderConfigInput") {
            inputFields { name }
          }
          updateSubtitleProviderInput: __type(name: "UpdateSubtitleProviderConfigInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          testSubtitleProviderInput: __type(name: "TestSubtitleProviderConnectionInput") {
            inputFields { name }
          }
          createNotificationChannelInput: __type(name: "CreateNotificationChannelInput") {
            inputFields { name }
          }
          updateNotificationChannelInput: __type(name: "UpdateNotificationChannelInput") {
            inputFields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let field_names = |type_alias: &str, field_key: &str| -> Vec<&str> {
        body["data"][type_alias][field_key]
            .as_array()
            .expect("type should expose fields")
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect()
    };

    assert_eq!(
        field_names("configValuePayload", "fields"),
        vec![
            "key",
            "label",
            "fieldType",
            "required",
            "defaultValue",
            "valueSource",
            "role",
            "hostBinding",
            "options",
            "helpText",
            "value"
        ]
    );
    assert_eq!(
        body["data"]["configFieldValue"]["possibleTypes"]
            .as_array()
            .expect("ProviderConfigFieldValue should expose possible types")
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect::<Vec<_>>(),
        vec![
            "StringConfigValuePayload",
            "BoolConfigValuePayload",
            "IntConfigValuePayload",
            "FloatConfigValuePayload",
            "SecretConfigValuePayload"
        ]
    );
    assert_eq!(
        field_names("configValueInput", "inputFields"),
        vec![
            "key",
            "stringValue",
            "boolValue",
            "intValue",
            "floatValue",
            "secretValue",
            "clearSecret"
        ]
    );

    let output_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    let input_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose input fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    let assert_non_null_id = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["name"], "ID", "{label}");
    };
    let assert_optional_id = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "SCALAR", "{label}");
        assert_eq!(field["type"]["name"], "ID", "{label}");
    };
    let assert_non_null_id_list = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["kind"], "LIST", "{label}");
        assert_eq!(
            field["type"]["ofType"]["ofType"]["kind"], "NON_NULL",
            "{label}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["ofType"]["name"], "ID",
            "{label}"
        );
    };
    let assert_non_null_string_list = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["kind"], "LIST", "{label}");
        assert_eq!(
            field["type"]["ofType"]["ofType"]["kind"], "NON_NULL",
            "{label}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["ofType"]["name"], "String",
            "{label}"
        );
    };
    let assert_optional_string = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "SCALAR", "{label}");
        assert_eq!(field["type"]["name"], "String", "{label}");
    };
    let assert_optional_bool = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "SCALAR", "{label}");
        assert_eq!(field["type"]["name"], "Boolean", "{label}");
    };

    assert_optional_string(
        input_field("configValueInput", "secretValue"),
        "ProviderConfigValueInput.secretValue",
    );
    assert_optional_bool(
        input_field("configValueInput", "clearSecret"),
        "ProviderConfigValueInput.clearSecret",
    );
    let value = output_field("configValuePayload", "value");
    assert_eq!(
        value["type"]["kind"], "UNION",
        "ProviderConfigValuePayload.value"
    );
    assert_eq!(
        value["type"]["name"], "ProviderConfigFieldValue",
        "ProviderConfigValuePayload.value"
    );
    let field_type = output_field("configValuePayload", "fieldType");
    assert_eq!(
        field_type["type"]["name"], "PluginConfigFieldTypeValue",
        "ProviderConfigValuePayload.fieldType"
    );
    let value_source = output_field("configValuePayload", "valueSource");
    assert_eq!(
        value_source["type"]["name"], "PluginConfigValueSourceValue",
        "ProviderConfigValuePayload.valueSource"
    );
    let options = output_field("configValuePayload", "options");
    assert_eq!(
        options["type"]["kind"], "NON_NULL",
        "ProviderConfigValuePayload.options"
    );
    assert_eq!(
        options["type"]["ofType"]["kind"], "LIST",
        "ProviderConfigValuePayload.options"
    );
    assert_eq!(
        options["type"]["ofType"]["ofType"]["ofType"]["name"], "PluginConfigFieldOptionPayload",
        "ProviderConfigValuePayload.options"
    );

    assert_non_null_id(
        output_field("indexerPayload", "id"),
        "IndexerConfigPayload.id",
    );
    assert_optional_id(
        output_field("indexerPayload", "managedParentConfigId"),
        "IndexerConfigPayload.managedParentConfigId",
    );
    assert_optional_id(
        output_field("indexerPayload", "downloadClientId"),
        "IndexerConfigPayload.downloadClientId",
    );
    assert_non_null_id(
        output_field("downloadClientPayload", "id"),
        "DownloadClientConfigPayload.id",
    );
    for type_alias in [
        "indexerPayload",
        "downloadClientPayload",
        "notificationChannelPayload",
    ] {
        assert_non_null_string_list(
            output_field(type_alias, "storedSecretKeys"),
            &format!("{type_alias}.storedSecretKeys"),
        );
    }
    for field_name in ["parentConfigId", "createdIds", "updatedIds", "deletedIds"] {
        let field = output_field("indexerSyncPayload", field_name);
        if field_name == "parentConfigId" {
            assert_non_null_id(field, "IndexerConfigSyncPayload.parentConfigId");
        } else {
            assert_non_null_id_list(field, field_name);
        }
    }
    assert_non_null_id(
        input_field("updateIndexerInput", "id"),
        "UpdateIndexerConfigInput.id",
    );
    assert_optional_id(
        input_field("createIndexerInput", "downloadClientId"),
        "CreateIndexerConfigInput.downloadClientId",
    );
    assert_optional_id(
        input_field("updateIndexerInput", "downloadClientId"),
        "UpdateIndexerConfigInput.downloadClientId",
    );
    assert_eq!(
        field_names("indexerMappingCatalog", "fields"),
        vec!["clients", "indexers", "providerCompatibility"]
    );
    assert_eq!(
        field_names("indexerProviderCompatibility", "fields"),
        vec![
            "providerType",
            "protocolFamilies",
            "supportsMapping",
            "compatibleClientIds"
        ]
    );
    assert_optional_id(
        input_field("testIndexerInput", "indexerId"),
        "TestIndexerConnectionInput.indexerId",
    );
    assert_non_null_id(
        input_field("updateDownloadClientInput", "id"),
        "UpdateDownloadClientConfigInput.id",
    );
    assert_optional_id(
        input_field("testDownloadClientInput", "id"),
        "TestDownloadClientConnectionInput.id",
    );
    let disabled_until = input_field("updateSubtitleProviderInput", "disabledUntil");
    assert_eq!(
        disabled_until["type"]["kind"], "SCALAR",
        "UpdateSubtitleProviderConfigInput.disabledUntil"
    );
    assert_eq!(
        disabled_until["type"]["name"], "DateTime",
        "UpdateSubtitleProviderConfigInput.disabledUntil"
    );
    assert_non_null_id_list(
        input_field("reorderDownloadClientInput", "ids"),
        "ReorderDownloadClientConfigsInput.ids",
    );
    assert_non_null_id_list(
        output_field("reorderDownloadClientPayload", "ids"),
        "ReorderDownloadClientConfigsPayload.ids",
    );

    for type_alias in [
        "indexerPayload",
        "downloadClientPayload",
        "notificationChannelPayload",
    ] {
        let fields = field_names(type_alias, "fields");
        assert!(fields.contains(&"config"));
        assert!(!fields.contains(&"configJson"));
    }

    for type_alias in [
        "createIndexerInput",
        "updateIndexerInput",
        "testIndexerInput",
        "createDownloadClientInput",
        "updateDownloadClientInput",
        "testDownloadClientInput",
        "createSubtitleProviderInput",
        "updateSubtitleProviderInput",
        "testSubtitleProviderInput",
        "createNotificationChannelInput",
        "updateNotificationChannelInput",
    ] {
        let fields = field_names(type_alias, "inputFields");
        assert!(fields.contains(&"config"));
        assert!(!fields.contains(&"configJson"));
    }
}

#[tokio::test]
async fn graphql_introspection_config_deletes_use_id_and_payload_results() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deleteIndexerInput: __type(name: "DeleteIndexerConfigInput") { name }
          deleteDownloadClientInput: __type(name: "DeleteDownloadClientConfigInput") { name }
          deleteSubtitleProviderInput: __type(name: "DeleteSubtitleProviderConfigInput") { name }
          deleteIndexerPayload: __type(name: "DeleteIndexerConfigPayload") {
            fields { name }
          }
          deleteDownloadClientPayload: __type(name: "DeleteDownloadClientConfigPayload") {
            fields { name }
          }
          deleteSubtitleProviderPayload: __type(name: "DeleteSubtitleProviderConfigPayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    assert!(body["data"]["deleteIndexerInput"].is_null());
    assert!(body["data"]["deleteDownloadClientInput"].is_null());
    assert!(body["data"]["deleteSubtitleProviderInput"].is_null());

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation = |name: &str| {
        mutation_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("mutation field should exist")
    };
    fn id_arg(field: &Value) -> &Value {
        field["args"]
            .as_array()
            .expect("mutation should expose args")
            .iter()
            .find(|arg| arg["name"] == "id")
            .expect("id arg should exist")
    }

    for (name, payload_name) in [
        ("deleteIndexerConfig", "DeleteIndexerConfigPayload"),
        (
            "deleteDownloadClientConfig",
            "DeleteDownloadClientConfigPayload",
        ),
        (
            "deleteSubtitleProviderConfig",
            "DeleteSubtitleProviderConfigPayload",
        ),
    ] {
        let mutation = mutation(name);
        assert_eq!(mutation["type"]["ofType"]["name"], payload_name);
        assert_eq!(id_arg(mutation)["type"]["kind"], "NON_NULL");
        assert_eq!(id_arg(mutation)["type"]["ofType"]["name"], "ID");
    }

    for (payload, expected_fields) in [
        ("deleteIndexerPayload", vec!["id"]),
        (
            "deleteDownloadClientPayload",
            vec!["id", "clearedIndexerMappingCount"],
        ),
        ("deleteSubtitleProviderPayload", vec!["id"]),
    ] {
        let field_names: Vec<&str> = body["data"][payload]["fields"]
            .as_array()
            .expect("delete payload should expose fields")
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect();
        assert_eq!(field_names, expected_fields);
    }
}

#[tokio::test]
async fn graphql_introspection_media_server_delete_uses_id_and_payload_result() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields {
              name
              args { name type { kind name ofType { kind name } } }
            }
          }
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deletePayload: __type(name: "DeleteMediaServerConnectionPayload") {
            fields { name }
          }
          mediaServerConnection: __type(name: "MediaServerConnectionPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          defaultLibraryGrant: __type(name: "MediaServerDefaultLibraryGrantPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          defaultLibraryGrantInput: __type(name: "MediaServerDefaultLibraryGrantInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          updateMediaServerConnectionInput: __type(name: "UpdateMediaServerConnectionInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          mediaServerUserGroup: __type(name: "MediaServerUserGroupPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let delete = mutation_fields
        .iter()
        .find(|field| field["name"] == "deleteMediaServerConnection")
        .expect("deleteMediaServerConnection should exist");
    assert_eq!(
        delete["type"]["ofType"]["name"],
        "DeleteMediaServerConnectionPayload"
    );
    let id_arg = delete["args"]
        .as_array()
        .expect("deleteMediaServerConnection should expose args")
        .iter()
        .find(|arg| arg["name"] == "id")
        .expect("id arg should exist");
    assert_eq!(id_arg["type"]["kind"], "NON_NULL");
    assert_eq!(id_arg["type"]["ofType"]["name"], "ID");

    let query_fields = body["data"]["queryRoot"]["fields"]
        .as_array()
        .expect("QueryRoot should expose fields");
    let query_arg = |field_name: &str, arg_name: &str| {
        query_fields
            .iter()
            .find(|field| field["name"] == field_name)
            .expect("query field should exist")["args"]
            .as_array()
            .expect("query field should expose args")
            .iter()
            .find(|arg| arg["name"] == arg_name)
            .expect("query arg should exist")
            .clone()
    };
    // mediaServerConnection (singular) removed in the 0.17.0 root-wave trim;
    // the plural mediaServerConnections + MediaServerConnectionPayload type stay.
    let field_name = "jellyfinServerUsers";
    let arg = query_arg(field_name, "connectionId");
    assert_eq!(arg["type"]["kind"], "NON_NULL", "{field_name}");
    assert_eq!(arg["type"]["ofType"]["name"], "ID", "{field_name}");

    let output_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    let input_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose input fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    for (type_alias, field_name) in [
        ("mediaServerConnection", "id"),
        ("defaultLibraryGrant", "libraryId"),
        ("mediaServerUserGroup", "connectionId"),
    ] {
        let field = output_field(type_alias, field_name);
        assert_eq!(
            field["type"]["kind"], "NON_NULL",
            "{type_alias}.{field_name}"
        );
        assert_eq!(
            field["type"]["ofType"]["name"], "ID",
            "{type_alias}.{field_name}"
        );
    }
    for field_name in ["createdAt", "updatedAt"] {
        let field = output_field("mediaServerConnection", field_name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{field_name}");
        assert_eq!(field["type"]["ofType"]["name"], "DateTime", "{field_name}");
    }
    for (type_alias, field_name) in [
        ("defaultLibraryGrantInput", "libraryId"),
        ("updateMediaServerConnectionInput", "id"),
    ] {
        let field = input_field(type_alias, field_name);
        assert_eq!(
            field["type"]["kind"], "NON_NULL",
            "{type_alias}.{field_name}"
        );
        assert_eq!(
            field["type"]["ofType"]["name"], "ID",
            "{type_alias}.{field_name}"
        );
    }

    let payload_fields: Vec<&str> = body["data"]["deletePayload"]["fields"]
        .as_array()
        .expect("DeleteMediaServerConnectionPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(payload_fields, vec!["id"]);
}

#[tokio::test]
async fn graphql_introspection_library_delete_uses_id_and_payload_result() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deleteInput: __type(name: "DeleteLibraryInput") { name }
          deletePayload: __type(name: "DeleteLibraryPayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    assert!(body["data"]["deleteInput"].is_null());

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let delete = mutation_fields
        .iter()
        .find(|field| field["name"] == "deleteLibrary")
        .expect("deleteLibrary should exist");
    assert_eq!(delete["type"]["ofType"]["name"], "DeleteLibraryPayload");
    let id_arg = delete["args"]
        .as_array()
        .expect("deleteLibrary should expose args")
        .iter()
        .find(|arg| arg["name"] == "id")
        .expect("id arg should exist");
    assert_eq!(id_arg["type"]["kind"], "NON_NULL");
    assert_eq!(id_arg["type"]["ofType"]["name"], "ID");

    let payload_fields: Vec<&str> = body["data"]["deletePayload"]["fields"]
        .as_array()
        .expect("DeleteLibraryPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(payload_fields, vec!["id"]);
}

#[tokio::test]
async fn graphql_introspection_media_file_delete_uses_payload_result() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deleteInput: __type(name: "DeleteMediaFileInput") {
            inputFields { name }
          }
          deletePayload: __type(name: "DeleteMediaFilePayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let input_fields: Vec<&str> = body["data"]["deleteInput"]["inputFields"]
        .as_array()
        .expect("DeleteMediaFileInput should remain an input object")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(
        input_fields,
        vec![
            "fileId",
            "deleteFromDisk",
            "previewFingerprint",
            "typedConfirmation"
        ]
    );

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let delete = mutation_fields
        .iter()
        .find(|field| field["name"] == "deleteMediaFile")
        .expect("deleteMediaFile should exist");
    assert_eq!(delete["type"]["ofType"]["name"], "DeleteMediaFilePayload");

    let payload_fields: Vec<&str> = body["data"]["deletePayload"]["fields"]
        .as_array()
        .expect("DeleteMediaFilePayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(payload_fields, vec!["id", "jobRun"]);
}

#[tokio::test]
async fn graphql_introspection_title_delete_uses_payload_result() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deleteInput: __type(name: "DeleteTitleInput") {
            inputFields { name }
          }
          deletePayload: __type(name: "DeleteTitlePayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let input_fields: Vec<&str> = body["data"]["deleteInput"]["inputFields"]
        .as_array()
        .expect("DeleteTitleInput should remain an input object")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(
        input_fields,
        vec![
            "titleId",
            "deleteFilesOnDisk",
            "previewFingerprint",
            "typedConfirmation"
        ]
    );

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let delete = mutation_fields
        .iter()
        .find(|field| field["name"] == "deleteTitle")
        .expect("deleteTitle should exist");
    assert_eq!(delete["type"]["ofType"]["name"], "DeleteTitlePayload");

    let payload_fields: Vec<&str> = body["data"]["deletePayload"]["fields"]
        .as_array()
        .expect("DeleteTitlePayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(payload_fields, vec!["id"]);
}

#[tokio::test]
async fn graphql_introspection_release_blocklist_clear_uses_id_and_payload_result() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          clearInput: __type(name: "ClearTitleReleaseBlocklistEntryInput") { name }
          clearPayload: __type(name: "ClearTitleReleaseBlocklistEntryPayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    assert!(body["data"]["clearInput"].is_null());

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let clear = mutation_fields
        .iter()
        .find(|field| field["name"] == "clearTitleReleaseBlocklistEntry")
        .expect("clearTitleReleaseBlocklistEntry should exist");
    assert_eq!(
        clear["type"]["ofType"]["name"],
        "ClearTitleReleaseBlocklistEntryPayload"
    );
    let id_arg = clear["args"]
        .as_array()
        .expect("clearTitleReleaseBlocklistEntry should expose args")
        .iter()
        .find(|arg| arg["name"] == "id")
        .expect("id arg should exist");
    assert_eq!(id_arg["type"]["kind"], "NON_NULL");
    assert_eq!(id_arg["type"]["ofType"]["name"], "ID");

    let payload_fields: Vec<&str> = body["data"]["clearPayload"]["fields"]
        .as_array()
        .expect("ClearTitleReleaseBlocklistEntryPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(payload_fields, vec!["id"]);
}

#[tokio::test]
async fn graphql_introspection_wanted_and_pending_actions_use_id_and_payload_results() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          wantedInput: __type(name: "WantedItemIdInput") { name }
          pendingInput: __type(name: "PendingReleaseActionInput") { name }
          pausePayload: __type(name: "PauseWantedItemPayload") { fields { name } }
          resumePayload: __type(name: "ResumeWantedItemPayload") { fields { name } }
          resetPayload: __type(name: "ResetWantedItemPayload") { name }
          forceGrabPayload: __type(name: "ForceGrabPendingReleasePayload") { fields { name } }
          dismissPayload: __type(name: "DismissPendingReleasePayload") { fields { name } }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    assert!(body["data"]["wantedInput"].is_null());
    assert!(body["data"]["pendingInput"].is_null());

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation = |name: &str| {
        mutation_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("mutation field should exist")
    };
    fn id_arg(field: &Value) -> &Value {
        field["args"]
            .as_array()
            .expect("mutation should expose args")
            .iter()
            .find(|arg| arg["name"] == "id")
            .expect("id arg should exist")
    }

    for (name, payload_name) in [
        ("pauseWantedItem", "PauseWantedItemPayload"),
        ("resumeWantedItem", "ResumeWantedItemPayload"),
        ("forceGrabPendingRelease", "ForceGrabPendingReleasePayload"),
        ("dismissPendingRelease", "DismissPendingReleasePayload"),
    ] {
        let field = mutation(name);
        assert_eq!(field["type"]["ofType"]["name"], payload_name);
        assert_eq!(id_arg(field)["type"]["kind"], "NON_NULL");
        assert_eq!(id_arg(field)["type"]["ofType"]["name"], "ID");
    }

    // cutover: `resetWantedItem` and its payload were removed — the
    // interactive search job (`triggerAcquisitionSearch`) owns re-search now.
    assert!(body["data"]["resetPayload"].is_null());
    assert!(
        !mutation_fields
            .iter()
            .any(|field| field["name"] == "resetWantedItem")
    );

    for (payload, flag) in [
        ("pausePayload", ""),
        ("resumePayload", ""),
        ("forceGrabPayload", "grabbed"),
        ("dismissPayload", ""),
    ] {
        let field_names: Vec<&str> = body["data"][payload]["fields"]
            .as_array()
            .expect("action payload should expose fields")
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect();
        let expected = if flag.is_empty() {
            vec!["id"]
        } else {
            vec!["id", flag]
        };
        assert_eq!(field_names, expected);
    }
}

#[tokio::test]
async fn graphql_introspection_rule_set_delete_uses_id_and_payload_result() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deletePayload: __type(name: "DeleteRuleSetPayload") {
            fields { name }
          }
          updateInput: __type(name: "UpdateRuleSetInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          toggleInput: __type(name: "ToggleRuleSetInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          validateInput: __type(name: "ValidateRuleSetInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          requiredAudioInput: __type(name: "SetTitleRequiredAudioInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let delete = mutation_fields
        .iter()
        .find(|field| field["name"] == "deleteRuleSet")
        .expect("deleteRuleSet should exist");
    assert_eq!(delete["type"]["ofType"]["name"], "DeleteRuleSetPayload");
    let id_arg = delete["args"]
        .as_array()
        .expect("deleteRuleSet should expose args")
        .iter()
        .find(|arg| arg["name"] == "id")
        .expect("id arg should exist");
    assert_eq!(id_arg["type"]["kind"], "NON_NULL");
    assert_eq!(id_arg["type"]["ofType"]["name"], "ID");

    let payload_fields: Vec<&str> = body["data"]["deletePayload"]["fields"]
        .as_array()
        .expect("DeleteRuleSetPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(payload_fields, vec!["id"]);

    let input_field = |type_alias: &str, name: &str| {
        body["data"][type_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose input fields"))
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("{type_alias}.{name} should exist"))
            .clone()
    };
    for (type_alias, name) in [
        ("updateInput", "id"),
        ("toggleInput", "id"),
        ("requiredAudioInput", "titleId"),
    ] {
        let field = input_field(type_alias, name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{type_alias}.{name}");
        assert_eq!(field["type"]["ofType"]["name"], "ID", "{type_alias}.{name}");
    }
    let rule_set_id = input_field("validateInput", "ruleSetId");
    assert_eq!(rule_set_id["type"]["kind"], "SCALAR");
    assert_eq!(rule_set_id["type"]["name"], "ID");
}

#[tokio::test]
async fn graphql_introspection_plugin_and_post_processing_ids_use_id_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          registryPlugin: __type(name: "RegistryPluginPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          pluginInstallation: __type(name: "PluginInstallationPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          pluginInstallProgress: __type(name: "PluginInstallProgressPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          togglePluginInput: __type(name: "TogglePluginInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          updatePostProcessingScriptInput: __type(name: "UpdatePostProcessingScriptInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let output_field = |type_alias: &str, name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("{type_alias}.{name} should exist"))
            .clone()
    };
    let input_field = |type_alias: &str, name: &str| {
        body["data"][type_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose input fields"))
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("{type_alias}.{name} should exist"))
            .clone()
    };
    let assert_non_null_id = |field: serde_json::Value, label: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["name"], "ID", "{label}");
    };

    for (type_alias, name) in [
        ("registryPlugin", "id"),
        ("pluginInstallation", "id"),
        ("pluginInstallation", "pluginId"),
        ("pluginInstallProgress", "pluginId"),
    ] {
        assert_non_null_id(
            output_field(type_alias, name),
            &format!("{type_alias}.{name}"),
        );
    }
    for (type_alias, name) in [
        ("togglePluginInput", "pluginId"),
        ("updatePostProcessingScriptInput", "id"),
    ] {
        assert_non_null_id(
            input_field(type_alias, name),
            &format!("{type_alias}.{name}"),
        );
    }
}

#[tokio::test]
async fn graphql_introspection_backup_actions_use_inputs_and_payload_results() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          createInput: __type(name: "CreateBackupInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          prepareInput: __type(name: "PrepareBackupDownloadInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          deleteInput: __type(name: "DeleteBackupInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          deletePayload: __type(name: "DeleteBackupPayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation = |name: &str| {
        mutation_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("mutation should exist")
    };
    let assert_input_arg = |mutation_name: &str, input_type: &str| {
        let args = mutation(mutation_name)["args"]
            .as_array()
            .expect("mutation should expose args");
        assert_eq!(args.len(), 1);
        assert_eq!(args[0]["name"], "input");
        assert_eq!(args[0]["type"]["kind"], "NON_NULL");
        assert_eq!(args[0]["type"]["ofType"]["name"], input_type);
    };
    assert_input_arg("createBackup", "CreateBackupInput");
    assert_input_arg("prepareBackupDownload", "PrepareBackupDownloadInput");
    assert_input_arg("deleteBackup", "DeleteBackupInput");

    let delete = mutation("deleteBackup");
    assert_eq!(delete["type"]["ofType"]["name"], "DeleteBackupPayload");

    let create_input_password = body["data"]["createInput"]["inputFields"]
        .as_array()
        .expect("CreateBackupInput should expose fields")
        .iter()
        .find(|field| field["name"] == "password")
        .expect("password field should exist");
    assert_eq!(create_input_password["type"]["kind"], "NON_NULL");
    assert_eq!(create_input_password["type"]["ofType"]["name"], "String");
    for (type_name, field_name) in [("prepareInput", "filename"), ("deleteInput", "filename")] {
        let filename_field = body["data"][type_name]["inputFields"]
            .as_array()
            .expect("backup filename input should expose fields")
            .iter()
            .find(|field| field["name"] == field_name)
            .expect("filename field should exist");
        assert_eq!(filename_field["type"]["kind"], "NON_NULL");
        assert_eq!(filename_field["type"]["ofType"]["name"], "String");
    }

    let payload_fields: Vec<&str> = body["data"]["deletePayload"]["fields"]
        .as_array()
        .expect("DeleteBackupPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(payload_fields, vec!["filename", "deleted"]);
}

#[tokio::test]
async fn graphql_introspection_subtitle_actions_use_payload_results() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          searchInput: __type(name: "SearchSubtitlesInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          searchPayload: __type(name: "SubtitleSearchResult") {
            fields { name type { kind name ofType { kind name } } }
          }
          downloadInput: __type(name: "DownloadSubtitleInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          deleteInput: __type(name: "DeleteExternalSubtitleInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          blocklistInput: __type(name: "BlocklistExternalSubtitleInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          updateProviderInput: __type(name: "UpdateSubtitleProviderConfigInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          testProviderInput: __type(name: "TestSubtitleProviderConnectionInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          downloadPayload: __type(name: "DownloadSubtitlePayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          deletePayload: __type(name: "DeleteExternalSubtitlePayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          blocklistPayload: __type(name: "BlocklistExternalSubtitlePayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          subtitleProviderConfig: __type(name: "SubtitleProviderConfigPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          externalSubtitlePayload: __type(name: "ExternalSubtitlePayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          externalSubtitleBlocklistEntryPayload: __type(name: "ExternalSubtitleBlocklistEntryPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation = |name: &str| {
        mutation_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("mutation field should exist")
    };
    let input_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose input fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    let assert_input_non_null = |type_alias: &str, field_name: &str, scalar_name: &str| {
        let field = input_field(type_alias, field_name);
        assert_eq!(
            field["type"]["kind"], "NON_NULL",
            "{type_alias}.{field_name}"
        );
        assert_eq!(
            field["type"]["ofType"]["name"], scalar_name,
            "{type_alias}.{field_name}"
        );
    };
    let assert_input_optional = |type_alias: &str, field_name: &str, scalar_name: &str| {
        let field = input_field(type_alias, field_name);
        assert_eq!(field["type"]["kind"], "SCALAR", "{type_alias}.{field_name}");
        assert_eq!(
            field["type"]["name"], scalar_name,
            "{type_alias}.{field_name}"
        );
    };
    let payload_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    let assert_payload_non_null = |type_alias: &str, field_name: &str, scalar_name: &str| {
        let field = payload_field(type_alias, field_name);
        assert_eq!(
            field["type"]["kind"], "NON_NULL",
            "{type_alias}.{field_name}"
        );
        assert_eq!(
            field["type"]["ofType"]["name"], scalar_name,
            "{type_alias}.{field_name}"
        );
    };
    let assert_payload_optional = |type_alias: &str, field_name: &str, scalar_name: &str| {
        let field = payload_field(type_alias, field_name);
        assert_eq!(field["type"]["kind"], "SCALAR", "{type_alias}.{field_name}");
        assert_eq!(
            field["type"]["name"], scalar_name,
            "{type_alias}.{field_name}"
        );
    };

    assert_input_non_null("searchInput", "mediaFileId", "ID");
    assert_input_non_null("searchInput", "language", "String");
    assert_payload_non_null("searchPayload", "score", "Int");
    assert_payload_non_null("searchPayload", "scorePercent", "Int");
    assert_input_non_null("downloadInput", "mediaFileId", "ID");
    assert_input_non_null("downloadInput", "providerFileId", "String");
    assert_input_non_null("downloadInput", "language", "String");
    assert_input_non_null("deleteInput", "externalSubtitleId", "ID");
    assert_input_non_null("blocklistInput", "externalSubtitleId", "ID");
    assert_input_non_null("updateProviderInput", "id", "ID");
    assert_input_optional("testProviderInput", "id", "ID");

    for (name, payload_name) in [
        ("downloadSubtitle", "DownloadSubtitlePayload"),
        ("deleteExternalSubtitle", "DeleteExternalSubtitlePayload"),
        (
            "blocklistExternalSubtitle",
            "BlocklistExternalSubtitlePayload",
        ),
    ] {
        assert_eq!(mutation(name)["type"]["ofType"]["name"], payload_name);
    }

    let download_fields: Vec<&str> = body["data"]["downloadPayload"]["fields"]
        .as_array()
        .expect("DownloadSubtitlePayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(
        download_fields,
        vec!["mediaFileId", "providerFileId", "downloaded"]
    );
    assert_payload_non_null("downloadPayload", "mediaFileId", "ID");
    assert_payload_non_null("downloadPayload", "providerFileId", "String");
    assert_payload_non_null("downloadPayload", "downloaded", "Boolean");

    let delete_fields: Vec<&str> = body["data"]["deletePayload"]["fields"]
        .as_array()
        .expect("DeleteExternalSubtitlePayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(delete_fields, vec!["id", "deleted"]);
    assert_payload_non_null("deletePayload", "id", "ID");
    assert_payload_non_null("deletePayload", "deleted", "Boolean");

    let blocklist_fields: Vec<&str> = body["data"]["blocklistPayload"]["fields"]
        .as_array()
        .expect("BlocklistExternalSubtitlePayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(blocklist_fields, vec!["id", "blocklisted"]);
    assert_payload_non_null("blocklistPayload", "id", "ID");
    assert_payload_non_null("blocklistPayload", "blocklisted", "Boolean");

    assert_payload_non_null("subtitleProviderConfig", "id", "ID");

    for field_name in ["id", "mediaFileId", "titleId"] {
        assert_payload_non_null("externalSubtitlePayload", field_name, "ID");
    }
    assert_payload_optional("externalSubtitlePayload", "episodeId", "ID");
    assert_payload_optional("externalSubtitlePayload", "providerFileId", "String");
    assert_payload_optional("externalSubtitlePayload", "score", "Int");
    assert_payload_optional("externalSubtitlePayload", "scorePercent", "Int");

    for field_name in ["id", "mediaFileId"] {
        assert_payload_non_null("externalSubtitleBlocklistEntryPayload", field_name, "ID");
    }
    assert_payload_non_null(
        "externalSubtitleBlocklistEntryPayload",
        "providerFileId",
        "String",
    );
}

#[tokio::test]
async fn graphql_introspection_title_acquisition_inputs_use_id_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          searchReleases: __type(name: "SearchReleasesInput") { inputFields { name type { ...TypeRef } } }
          queueDownload: __type(name: "QueueDownloadInput") { inputFields { name type { ...TypeRef } } }
          queueBestRelease: __type(name: "QueueBestReleaseInput") { inputFields { name type { ...TypeRef } } }
          queueDownloadScope: __type(name: "QueueDownloadScopeInput") { inputFields { name type { ...TypeRef } } }
          retryImport: __type(name: "RetryImportInput") { inputFields { name type { ...TypeRef } } }
          ignoreTrackedDownload: __type(name: "IgnoreTrackedDownloadInput") { inputFields { name type { ...TypeRef } } }
          markTrackedDownloadFailed: __type(name: "MarkTrackedDownloadFailedInput") { inputFields { name type { ...TypeRef } } }
          assignTrackedDownloadTitle: __type(name: "AssignTrackedDownloadTitleInput") { inputFields { name type { ...TypeRef } } }
          resolvePendingImport: __type(name: "ResolvePendingImportInput") { inputFields { name type { ...TypeRef } } }
          bindPendingImport: __type(name: "BindPendingImportInput") { inputFields { name type { ...TypeRef } } }
          triggerAcquisitionSearch: __type(name: "TriggerAcquisitionSearchInput") { inputFields { name type { ...TypeRef } } }
          triggerWantedSearch: __type(name: "TriggerWantedSearchInput") { name }
          triggerTitleWantedSearch: __type(name: "TriggerTitleWantedSearchInput") { name }
          triggerSeasonWantedSearch: __type(name: "TriggerSeasonWantedSearchInput") { name }
          deleteTitle: __type(name: "DeleteTitleInput") { inputFields { name type { ...TypeRef } } }
          deleteTitlesItem: __type(name: "DeleteTitlesItemInput") { inputFields { name type { ...TypeRef } } }
          deleteTitlesPreview: __type(name: "DeleteTitlesPreviewInput") { inputFields { name type { ...TypeRef } } }
          deleteEpisodeFiles: __type(name: "DeleteEpisodeFilesInput") { inputFields { name type { ...TypeRef } } }
          deleteEpisodeFilesPreview: __type(name: "DeleteEpisodeFilesPreviewInput") { inputFields { name type { ...TypeRef } } }
          setTitleMonitored: __type(name: "SetTitleMonitoredInput") { inputFields { name type { ...TypeRef } } }
          updateTitle: __type(name: "UpdateTitleInput") { inputFields { name type { ...TypeRef } } }
          setPrimaryMovieFile: __type(name: "SetPrimaryMovieFileInput") { inputFields { name type { ...TypeRef } } }
          fixTitleMatch: __type(name: "FixTitleMatchInput") { inputFields { name type { ...TypeRef } } }
          setCollectionMonitored: __type(name: "SetCollectionMonitoredInput") { inputFields { name type { ...TypeRef } } }
          setEpisodeMonitored: __type(name: "SetEpisodeMonitoredInput") { inputFields { name type { ...TypeRef } } }
          setSeriesMovieMonitored: __type(name: "SetSeriesMovieMonitoredInput") { inputFields { name type { ...TypeRef } } }
          deleteMediaFile: __type(name: "DeleteMediaFileInput") { inputFields { name type { ...TypeRef } } }
          manualImportCandidateMapping: __type(name: "ManualImportCandidateMappingInput") { inputFields { name type { ...TypeRef } } }
          beginManualImportSelection: __type(name: "BeginManualImportSelectionInput") { inputFields { name type { ...TypeRef } } }
          queueManualImport: __type(name: "QueueManualImportInput") { inputFields { name type { ...TypeRef } } }
          pauseDownload: __type(name: "PauseDownloadInput") { inputFields { name type { ...TypeRef } } }
          resumeDownload: __type(name: "ResumeDownloadInput") { inputFields { name type { ...TypeRef } } }
          deleteDownload: __type(name: "DeleteDownloadInput") { inputFields { name type { ...TypeRef } } }
          mediaRenamePreview: __type(name: "MediaRenamePreviewInput") { inputFields { name type { ...TypeRef } } }
          mediaRenameApply: __type(name: "MediaRenameApplyInput") { inputFields { name type { ...TypeRef } } }
        }

        fragment TypeRef on __Type {
          kind
          name
          ofType {
            kind
            name
            ofType {
              kind
              name
              ofType {
                kind
                name
              }
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let input_field = |input_alias: &str, field_name: &str| {
        body["data"][input_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{input_alias} should expose input fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{input_alias}.{field_name} should exist"))
            .clone()
    };
    let assert_non_null_id = |input_alias: &str, field_name: &str| {
        let field = input_field(input_alias, field_name);
        assert_eq!(
            field["type"]["kind"], "NON_NULL",
            "{input_alias}.{field_name}"
        );
        assert_eq!(
            field["type"]["ofType"]["name"], "ID",
            "{input_alias}.{field_name}"
        );
    };
    let assert_nullable_id = |input_alias: &str, field_name: &str| {
        let field = input_field(input_alias, field_name);
        assert_eq!(
            field["type"]["kind"], "SCALAR",
            "{input_alias}.{field_name}"
        );
        assert_eq!(field["type"]["name"], "ID", "{input_alias}.{field_name}");
    };
    let assert_non_null_id_list = |input_alias: &str, field_name: &str| {
        let field = input_field(input_alias, field_name);
        assert_eq!(
            field["type"]["kind"], "NON_NULL",
            "{input_alias}.{field_name}"
        );
        assert_eq!(
            field["type"]["ofType"]["kind"], "LIST",
            "{input_alias}.{field_name}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["kind"], "NON_NULL",
            "{input_alias}.{field_name}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["ofType"]["name"], "ID",
            "{input_alias}.{field_name}"
        );
    };
    let assert_nullable_id_list = |input_alias: &str, field_name: &str| {
        let field = input_field(input_alias, field_name);
        assert_eq!(field["type"]["kind"], "LIST", "{input_alias}.{field_name}");
        assert_eq!(
            field["type"]["ofType"]["kind"], "NON_NULL",
            "{input_alias}.{field_name}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["name"], "ID",
            "{input_alias}.{field_name}"
        );
    };

    for (input_alias, field_name) in [
        ("queueDownload", "titleId"),
        ("queueBestRelease", "titleId"),
        ("retryImport", "importId"),
        ("assignTrackedDownloadTitle", "titleId"),
        ("resolvePendingImport", "pendingImportId"),
        ("bindPendingImport", "pendingImportId"),
        ("deleteTitle", "titleId"),
        ("deleteTitlesItem", "titleId"),
        ("setTitleMonitored", "titleId"),
        ("updateTitle", "titleId"),
        ("setPrimaryMovieFile", "titleId"),
        ("setPrimaryMovieFile", "fileId"),
        ("fixTitleMatch", "titleId"),
        ("setCollectionMonitored", "collectionId"),
        ("setEpisodeMonitored", "episodeId"),
        ("setSeriesMovieMonitored", "seriesMovieLinkId"),
        ("deleteMediaFile", "fileId"),
        ("deleteEpisodeFiles", "titleId"),
        ("deleteEpisodeFilesPreview", "titleId"),
        ("beginManualImportSelection", "clientId"),
        ("beginManualImportSelection", "titleId"),
        ("queueManualImport", "selectionId"),
        ("mediaRenameApply", "titleId"),
    ] {
        assert_non_null_id(input_alias, field_name);
    }

    for (input_alias, field_name) in [
        // Nullable since spec 0002: a search names either a title or a query.
        ("searchReleases", "titleId"),
        ("searchReleases", "seriesMovieLinkId"),
        ("pauseDownload", "clientId"),
        ("resumeDownload", "clientId"),
        ("deleteDownload", "clientId"),
        ("ignoreTrackedDownload", "clientId"),
        ("markTrackedDownloadFailed", "clientId"),
        ("assignTrackedDownloadTitle", "clientId"),
        ("bindPendingImport", "collectionId"),
        ("manualImportCandidateMapping", "episodeId"),
        ("manualImportCandidateMapping", "seriesMovieLinkId"),
        ("mediaRenamePreview", "titleId"),
        ("queueDownloadScope", "episode"),
        ("queueDownloadScope", "seriesMovie"),
        ("queueDownloadScope", "collection"),
    ] {
        assert_nullable_id(input_alias, field_name);
    }

    assert_non_null_id_list("deleteTitlesPreview", "titleIds");
    assert_non_null_id_list("deleteEpisodeFiles", "episodeIds");
    assert_non_null_id_list("deleteEpisodeFilesPreview", "episodeIds");
    assert_non_null_id_list("bindPendingImport", "episodeIds");
    assert_nullable_id_list("queueDownloadScope", "episodeSet");

    // The interactive search job input replaces the per-item trigger
    // inputs; its scoping ids are all optional.
    assert_nullable_id("triggerAcquisitionSearch", "titleId");
    assert_nullable_id("triggerAcquisitionSearch", "wantedItemId");
    assert_nullable_id_list("triggerAcquisitionSearch", "libraryIds");
    assert!(body["data"]["triggerWantedSearch"].is_null());
    assert!(body["data"]["triggerTitleWantedSearch"].is_null());
    assert!(body["data"]["triggerSeasonWantedSearch"].is_null());
}

#[tokio::test]
async fn graphql_introspection_external_import_finalize_uses_payload_results() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          cancelInput: __type(name: "CancelExternalImportMonitorWarmupInput") { name }
          sourceWarmupInput: __type(name: "StartExternalImportArrSourceWarmupInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          cancelPayload: __type(name: "CancelExternalImportMonitorWarmupPayload") {
            fields { name }
          }
          finalizePayload: __type(name: "FinalizeExternalImportPayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    assert!(body["data"]["cancelInput"].is_null());

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation = |name: &str| {
        mutation_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("mutation field should exist")
    };
    let mutation_arg = |field_name: &str, arg_name: &str| {
        mutation(field_name)["args"]
            .as_array()
            .unwrap_or_else(|| panic!("{field_name} should expose args"))
            .iter()
            .find(|arg| arg["name"] == arg_name)
            .unwrap_or_else(|| panic!("{field_name}.{arg_name} should exist"))
            .clone()
    };

    let start = mutation("startExternalImportArrSourceWarmup");
    assert_eq!(
        start["type"]["ofType"]["name"],
        "ExternalImportMonitorWarmupProgressPayload"
    );
    let start_input_arg = mutation_arg("startExternalImportArrSourceWarmup", "input");
    assert_eq!(start_input_arg["type"]["kind"], "NON_NULL");
    assert_eq!(
        start_input_arg["type"]["ofType"]["name"],
        "StartExternalImportArrSourceWarmupInput"
    );

    let cancel = mutation("cancelExternalImportArrSourceWarmup");
    assert_eq!(
        cancel["type"]["ofType"]["name"],
        "CancelExternalImportMonitorWarmupPayload"
    );
    let session_id_arg = mutation_arg("cancelExternalImportArrSourceWarmup", "sessionId");
    assert_eq!(session_id_arg["type"]["kind"], "NON_NULL");
    assert_eq!(session_id_arg["type"]["ofType"]["name"], "ID");

    let finalize = mutation("finalizeExternalImport");
    assert_eq!(
        finalize["type"]["ofType"]["name"],
        "FinalizeExternalImportPayload"
    );
    let finalize_input_arg = mutation_arg("finalizeExternalImport", "input");
    assert_eq!(finalize_input_arg["type"]["kind"], "NON_NULL");
    assert_eq!(
        finalize_input_arg["type"]["ofType"]["name"],
        "FinalizeExternalImportInput"
    );

    let source_warmup_fields = body["data"]["sourceWarmupInput"]["inputFields"]
        .as_array()
        .expect("StartExternalImportArrSourceWarmupInput should expose fields");
    let source_warmup_field = |name: &str| {
        source_warmup_fields
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| {
                panic!("StartExternalImportArrSourceWarmupInput.{name} should exist")
            })
            .clone()
    };
    let kind = source_warmup_field("kind");
    assert_eq!(kind["type"]["kind"], "NON_NULL");
    assert_eq!(kind["type"]["ofType"]["name"], "ExternalArrSourceKind");
    let connection = source_warmup_field("connection");
    assert_eq!(connection["type"]["kind"], "NON_NULL");
    assert_eq!(
        connection["type"]["ofType"]["name"],
        "ExternalImportConnectionInput"
    );

    let cancel_fields: Vec<&str> = body["data"]["cancelPayload"]["fields"]
        .as_array()
        .expect("CancelExternalImportMonitorWarmupPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(cancel_fields, vec!["sessionId", "canceled"]);

    let finalize_fields: Vec<&str> = body["data"]["finalizePayload"]["fields"]
        .as_array()
        .expect("FinalizeExternalImportPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(finalize_fields, vec!["monitorWarmupSessionId"]);
}

fn graphql_type_leaf_name(type_value: &Value) -> Option<&str> {
    let mut current = type_value;
    loop {
        if let Some(name) = current["name"].as_str() {
            return Some(name);
        }
        current = &current["ofType"];
        if current.is_null() {
            return None;
        }
    }
}

fn introspection_names(body: &Value, type_alias: &str, field_key: &str) -> Vec<String> {
    body["data"][type_alias][field_key]
        .as_array()
        .unwrap_or_else(|| panic!("{type_alias} should expose {field_key}"))
        .iter()
        .filter_map(|field| field["name"].as_str().map(str::to_string))
        .collect()
}

fn introspection_entry<'a>(
    body: &'a Value,
    type_alias: &str,
    field_key: &str,
    name: &str,
) -> &'a Value {
    body["data"][type_alias][field_key]
        .as_array()
        .unwrap_or_else(|| panic!("{type_alias} should expose {field_key}"))
        .iter()
        .find(|field| field["name"] == name)
        .unwrap_or_else(|| panic!("{type_alias}.{name} should exist"))
}

#[tokio::test]
async fn graphql_introspection_external_import_secret_draft_api_is_typed() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields { name }
          }
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          saveInput: __type(name: "SaveExternalImportSetupSecretDraftInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          instanceInput: __type(name: "ExternalImportSetupInstanceApiKeyInput") {
            inputFields { name }
          }
          draftPayload: __type(name: "ExternalImportSetupSecretDraftPayload") {
            fields { name }
          }
          statusPayload: __type(name: "ExternalImportSetupSecretDraftStatusPayload") {
            fields { name }
          }
          savePayload: __type(name: "SaveExternalImportSetupSecretDraftPayload") {
            fields { name }
          }
          clearPayload: __type(name: "ClearExternalImportSetupSecretDraftPayload") {
            fields { name }
          }
          instancePayload: __type(name: "ExternalImportSetupInstanceApiKeyPayload") {
            fields { name }
          }
          apiKeyOverridePayload: __type(name: "ExternalImportSetupApiKeyOverridePayload") {
            fields { name }
          }
          passwordOverridePayload: __type(name: "ExternalImportSetupPasswordOverridePayload") {
            fields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let query_names = introspection_names(&body, "queryRoot", "fields");
    assert!(query_names.contains(&"externalImportSetupSecretDraft".to_string()));
    assert!(query_names.contains(&"externalImportSetupSecretDraftStatus".to_string()));

    let mutation_names = introspection_names(&body, "mutationRoot", "fields");
    assert!(mutation_names.contains(&"saveExternalImportSetupSecretDraft".to_string()));
    assert!(mutation_names.contains(&"clearExternalImportSetupSecretDraft".to_string()));

    let save_mutation = introspection_entry(
        &body,
        "mutationRoot",
        "fields",
        "saveExternalImportSetupSecretDraft",
    );
    let save_input_arg = save_mutation["args"]
        .as_array()
        .expect("save mutation args")
        .iter()
        .find(|arg| arg["name"] == "input")
        .expect("save input arg");
    assert_eq!(
        graphql_type_leaf_name(&save_input_arg["type"]),
        Some("SaveExternalImportSetupSecretDraftInput")
    );

    let clear_mutation = introspection_entry(
        &body,
        "mutationRoot",
        "fields",
        "clearExternalImportSetupSecretDraft",
    );
    assert!(
        clear_mutation["args"]
            .as_array()
            .expect("clear mutation args")
            .is_empty()
    );

    assert_eq!(
        introspection_names(&body, "saveInput", "inputFields"),
        vec![
            "instanceApiKeys",
            "downloadClientApiKeyOverrides",
            "downloadClientPasswordOverrides",
            "indexerApiKeyOverrides",
        ]
    );
    for (field_name, expected_item_type) in [
        ("instanceApiKeys", "ExternalImportSetupInstanceApiKeyInput"),
        (
            "downloadClientApiKeyOverrides",
            "DownloadClientApiKeyOverrideInput",
        ),
        (
            "downloadClientPasswordOverrides",
            "DownloadClientPasswordOverrideInput",
        ),
        ("indexerApiKeyOverrides", "IndexerApiKeyOverrideInput"),
    ] {
        let field = introspection_entry(&body, "saveInput", "inputFields", field_name);
        assert_eq!(field["type"]["kind"], "NON_NULL", "{field}");
        assert_eq!(field["type"]["ofType"]["kind"], "LIST", "{field}");
        assert_eq!(
            graphql_type_leaf_name(&field["type"]),
            Some(expected_item_type)
        );
    }

    assert_eq!(
        introspection_names(&body, "instanceInput", "inputFields"),
        vec!["instanceId", "kind", "apiKey"]
    );
    assert_eq!(
        introspection_names(&body, "draftPayload", "fields"),
        vec![
            "instanceApiKeys",
            "downloadClientApiKeyOverrides",
            "downloadClientPasswordOverrides",
            "indexerApiKeyOverrides",
            "updatedAt",
        ]
    );
    assert_eq!(
        introspection_names(&body, "statusPayload", "fields"),
        vec!["hasDraft", "ownedByCurrentUser", "updatedAt"]
    );
    assert_eq!(
        introspection_names(&body, "savePayload", "fields"),
        vec!["overwroteAnotherUserDraft", "updatedAt"]
    );
    assert_eq!(
        introspection_names(&body, "clearPayload", "fields"),
        vec!["cleared"]
    );
    assert_eq!(
        introspection_names(&body, "instancePayload", "fields"),
        vec!["instanceId", "kind", "apiKey"]
    );
    assert_eq!(
        introspection_names(&body, "apiKeyOverridePayload", "fields"),
        vec!["dedupKey", "apiKey"]
    );
    assert_eq!(
        introspection_names(&body, "passwordOverridePayload", "fields"),
        vec!["dedupKey", "password"]
    );

    for type_alias in [
        "saveInput",
        "instanceInput",
        "draftPayload",
        "statusPayload",
        "savePayload",
        "clearPayload",
        "instancePayload",
        "apiKeyOverridePayload",
        "passwordOverridePayload",
    ] {
        let names = if body["data"][type_alias]["inputFields"].is_array() {
            introspection_names(&body, type_alias, "inputFields")
        } else {
            introspection_names(&body, type_alias, "fields")
        };
        for name in names {
            assert_ne!(
                name, "draftJson",
                "{type_alias} should not expose draftJson"
            );
            assert!(
                !name.to_ascii_lowercase().contains("json"),
                "{type_alias}.{name} should not expose opaque JSON fields"
            );
        }
    }
}

#[tokio::test]
async fn graphql_introspection_external_import_does_not_project_api_keys() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          downloadClient: __type(name: "ExternalImportDownloadClientPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          indexer: __type(name: "ExternalImportIndexerPayload") {
            fields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = |type_alias: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
    };
    let field = |type_alias: &str, name: &str| {
        fields(type_alias)
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("{type_alias}.{name} should exist"))
            .clone()
    };
    for type_alias in ["downloadClient", "indexer"] {
        let names: Vec<&str> = fields(type_alias)
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect();
        assert!(
            !names.contains(&"apiKey"),
            "{type_alias} should not project API keys"
        );
        let api_key_present = field(type_alias, "apiKeyPresent");
        assert_eq!(
            api_key_present["type"]["kind"], "NON_NULL",
            "{type_alias}.apiKeyPresent"
        );
        assert_eq!(
            api_key_present["type"]["ofType"]["name"], "Boolean",
            "{type_alias}.apiKeyPresent"
        );
        let source_keys = field(type_alias, "sourceKeys");
        assert_eq!(
            source_keys["type"]["kind"], "NON_NULL",
            "{type_alias}.sourceKeys"
        );
        assert_eq!(
            source_keys["type"]["ofType"]["kind"], "LIST",
            "{type_alias}.sourceKeys"
        );
        assert_eq!(
            source_keys["type"]["ofType"]["ofType"]["kind"], "NON_NULL",
            "{type_alias}.sourceKeys"
        );
        assert_eq!(
            source_keys["type"]["ofType"]["ofType"]["ofType"]["name"], "String",
            "{type_alias}.sourceKeys"
        );
    }
}

#[tokio::test]
async fn graphql_introspection_external_import_warmup_uses_session_ids() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields(includeDeprecated: true) {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                      ofType {
                        kind
                        name
                      }
                    }
                  }
                }
              }
            }
          }
          subscriptionRoot: __type(name: "SubscriptionRoot") {
            fields { name args { name type { kind name ofType { kind name } } } }
          }
          previewInput: __type(name: "PreviewExternalImportInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          executeInput: __type(name: "ExecuteExternalImportInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          finalizeInput: __type(name: "FinalizeExternalImportInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          aggregateProgressInput: __type(name: "ExternalImportAggregateWarmupProgressInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          mappingInput: __type(name: "ExternalImportSourceLibraryMappingInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                  ofType {
                    kind
                    name
                    ofType {
                      kind
                      name
                    }
                  }
                }
              }
            }
          }
          progressPayload: __type(name: "ExternalImportMonitorWarmupProgressPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          aggregateProgressPayload: __type(name: "ExternalImportAggregateWarmupProgressPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let root_arg = |root_alias: &str, field_name: &str, arg_name: &str| {
        body["data"][root_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{root_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{field_name} should exist"))["args"]
            .as_array()
            .expect("field should expose args")
            .iter()
            .find(|arg| arg["name"] == arg_name)
            .unwrap_or_else(|| panic!("{field_name}.{arg_name} should exist"))
            .clone()
    };
    let root_has_field = |root_alias: &str, field_name: &str| {
        body["data"][root_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{root_alias} should expose fields"))
            .iter()
            .any(|field| field["name"] == field_name)
    };
    assert!(
        !root_has_field("queryRoot", "externalImportMonitorWarmupStatus"),
        "legacy externalImportMonitorWarmupStatus query should not exist"
    );
    // externalImportArrSourceWarmupStatus is deprecated (superseded by the
    // kind-neutral externalImportWarmupStatus) but must keep its contract
    // until removal — hence includeDeprecated: true on the queryRoot fields.
    for (root_alias, field_name) in [
        ("queryRoot", "externalImportArrSourceWarmupStatus"),
        ("queryRoot", "externalImportWarmupStatus"),
        ("subscriptionRoot", "externalImportMonitorWarmupProgress"),
    ] {
        let arg = root_arg(root_alias, field_name, "sessionId");
        assert_eq!(arg["type"]["kind"], "NON_NULL", "{field_name}");
        assert_eq!(arg["type"]["ofType"]["name"], "ID", "{field_name}");
    }

    let aggregate_arg = root_arg(
        "queryRoot",
        "externalImportAggregateWarmupProgress",
        "input",
    );
    assert_eq!(aggregate_arg["type"]["kind"], "NON_NULL");
    assert_eq!(
        aggregate_arg["type"]["ofType"]["name"],
        "ExternalImportAggregateWarmupProgressInput"
    );

    let input_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose input fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    let assert_non_null_id = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["name"], "ID", "{label}");
    };
    let assert_non_null_string = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["name"], "String", "{label}");
    };
    let assert_non_null_named = |field: Value, label: &str, name: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["name"], name, "{label}");
    };
    let assert_non_null_list = |field: Value, label: &str, item_name: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["kind"], "LIST", "{label}");
        assert_eq!(
            field["type"]["ofType"]["ofType"]["kind"], "NON_NULL",
            "{label}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["ofType"]["name"], item_name,
            "{label}"
        );
    };
    // Nullable (manual-root) fields surface the named type directly, not wrapped
    // in NON_NULL.
    let assert_nullable_named = |field: Value, label: &str, kind: &str, name: &str| {
        assert_eq!(field["type"]["kind"], kind, "{label}");
        assert_eq!(field["type"]["name"], name, "{label}");
    };

    for type_alias in [
        "previewInput",
        "executeInput",
        "finalizeInput",
        "aggregateProgressInput",
    ] {
        assert_non_null_list(
            input_field(type_alias, "sourceWarmupSessionIds"),
            &format!("{type_alias}.sourceWarmupSessionIds"),
            "ID",
        );
    }
    assert_non_null_list(
        input_field("finalizeInput", "mappings"),
        "finalizeInput.mappings",
        "ExternalImportSourceLibraryMappingInput",
    );

    // Manual-root support: these three are nullable (absent for a manually
    // added root that no warmup surfaced).
    assert_nullable_named(
        input_field("mappingInput", "sourceWarmupSessionId"),
        "mappingInput.sourceWarmupSessionId",
        "SCALAR",
        "ID",
    );
    assert_nullable_named(
        input_field("mappingInput", "sourceKey"),
        "mappingInput.sourceKey",
        "SCALAR",
        "String",
    );
    assert_nullable_named(
        input_field("mappingInput", "kind"),
        "mappingInput.kind",
        "ENUM",
        "ExternalArrSourceKind",
    );
    assert_non_null_string(
        input_field("mappingInput", "arrRootPath"),
        "mappingInput.arrRootPath",
    );
    assert_non_null_string(
        input_field("mappingInput", "scryerRootPath"),
        "mappingInput.scryerRootPath",
    );
    assert_non_null_id(
        input_field("mappingInput", "libraryId"),
        "mappingInput.libraryId",
    );
    assert_non_null_named(
        input_field("mappingInput", "facet"),
        "mappingInput.facet",
        "MediaFacetValue",
    );

    let payload_field = body["data"]["progressPayload"]["fields"]
        .as_array()
        .expect("ExternalImportMonitorWarmupProgressPayload should expose fields")
        .iter()
        .find(|field| field["name"] == "sessionId")
        .expect("sessionId should exist");
    assert_eq!(payload_field["type"]["kind"], "NON_NULL");
    assert_eq!(payload_field["type"]["ofType"]["name"], "ID");

    let aggregate_payload_fields: Vec<&str> = body["data"]["aggregateProgressPayload"]["fields"]
        .as_array()
        .expect("ExternalImportAggregateWarmupProgressPayload should expose fields")
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert_eq!(
        aggregate_payload_fields,
        vec![
            "status",
            "titlesTotalKnown",
            "titlesFetched",
            "titlesTotal",
            "errorMessage"
        ]
    );
}

#[tokio::test]
async fn graphql_introspection_account_setup_and_settings_actions_use_semantic_payloads() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args {
                name
                type {
                  kind
                  name
                  ofType {
                    kind
                    name
                  }
                }
              }
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deleteQualityProfileInput: __type(name: "DeleteQualityProfileInput") { name }
          deleteDelayProfileInput: __type(name: "DeleteDelayProfileInput") { name }
          deleteUserInput: __type(name: "DeleteUserInput") { name }
          resetUserMfaInput: __type(name: "ResetUserMfaInput") { name }
          unlinkExternalAccountInput: __type(name: "UnlinkExternalAccountInput") { name }
          titleIdInput: __type(name: "TitleIdInput") { name }
          cancelLibraryScanInput: __type(name: "CancelLibraryScanInput") { name }
          ignorePendingImportInput: __type(name: "IgnorePendingImportInput") { name }
          loginWithPlexInput: __type(name: "LoginWithPlexInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          loginWithJellyfinInput: __type(name: "LoginWithJellyfinInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          linkPlexAccountInput: __type(name: "LinkPlexAccountInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          linkJellyfinAccountInput: __type(name: "LinkJellyfinAccountInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          createExternalAccountInviteInput: __type(name: "CreateExternalAccountInviteInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          setUserPasswordInput: __type(name: "SetUserPasswordInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          setUserAppPermissionsInput: __type(name: "SetUserAppPermissionsInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          libraryPermissionGrantInput: __type(name: "LibraryPermissionGrantInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          setUserLibraryPermissionsInput: __type(name: "SetUserLibraryPermissionsInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          updateLibraryInput: __type(name: "UpdateLibraryInput") {
            inputFields { name type { kind name ofType { kind name } } }
          }
          externalAuthRuntimeConnection: __type(name: "ExternalAuthRuntimeConnectionPayload") {
            fields { name type { kind name ofType { kind name } } }
          }
          scanLibraryInput: __type(name: "ScanLibraryInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          rehydrateAllMetadataInput: __type(name: "RehydrateAllMetadataInput") {
            inputFields {
              name
              type {
                kind
                name
                ofType {
                  kind
                  name
                }
              }
            }
          }
          deleteMyPasskeyPayload: __type(name: "DeleteMyPasskeyPayload") { fields { name } }
          revokeMyOauthAppPayload: __type(name: "RevokeMyOauthAppPayload") { fields { name } }
          deleteUserPayload: __type(name: "DeleteUserPayload") { fields { name } }
          unlinkExternalAccountPayload: __type(name: "UnlinkExternalAccountPayload") { fields { name } }
          clearTitleImageCachePayload: __type(name: "ClearTitleImageCachePayload") { fields { name } }
          completeSetupPayload: __type(name: "CompleteSetupPayload") { fields { name } }
          reorderDownloadClientConfigsPayload: __type(name: "ReorderDownloadClientConfigsPayload") { fields { name } }
          setTitleRequiredAudioPayload: __type(name: "SetTitleRequiredAudioPayload") { fields { name } }
          rehydrateAllMetadataPayload: __type(name: "RehydrateAllMetadataPayload") { fields { name } }
          deletePostProcessingScriptPayload: __type(name: "DeletePostProcessingScriptPayload") { fields { name } }
          triggerTitleMismatchRecoverySearchPayload: __type(name: "TriggerTitleMismatchRecoverySearchPayload") { fields { name } }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    for removed_input in [
        "deleteQualityProfileInput",
        "deleteDelayProfileInput",
        "deleteUserInput",
        "resetUserMfaInput",
        "unlinkExternalAccountInput",
        "titleIdInput",
        "cancelLibraryScanInput",
        "ignorePendingImportInput",
    ] {
        assert!(
            body["data"][removed_input].is_null(),
            "{removed_input} should be removed from the schema"
        );
    }

    let auth_input_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose input fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    for (type_alias, field_name) in [
        ("loginWithPlexInput", "connectionId"),
        ("loginWithJellyfinInput", "connectionId"),
        ("linkPlexAccountInput", "connectionId"),
        ("linkJellyfinAccountInput", "connectionId"),
        ("createExternalAccountInviteInput", "userId"),
        ("createExternalAccountInviteInput", "connectionId"),
        ("setUserPasswordInput", "userId"),
        ("setUserAppPermissionsInput", "userId"),
        ("libraryPermissionGrantInput", "libraryId"),
        ("setUserLibraryPermissionsInput", "userId"),
        ("updateLibraryInput", "libraryId"),
    ] {
        let field = auth_input_field(type_alias, field_name);
        assert_eq!(
            field["type"]["kind"], "NON_NULL",
            "{type_alias}.{field_name}"
        );
        assert_eq!(
            field["type"]["ofType"]["name"], "ID",
            "{type_alias}.{field_name}"
        );
    }
    let runtime_connection_id = body["data"]["externalAuthRuntimeConnection"]["fields"]
        .as_array()
        .expect("ExternalAuthRuntimeConnectionPayload should expose fields")
        .iter()
        .find(|field| field["name"] == "id")
        .expect("ExternalAuthRuntimeConnectionPayload.id should exist");
    assert_eq!(runtime_connection_id["type"]["kind"], "NON_NULL");
    assert_eq!(runtime_connection_id["type"]["ofType"]["name"], "ID");

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation = |name: &str| {
        mutation_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("mutation field should exist")
    };
    let assert_id_arg = |mutation_name: &str, arg_name: &str| {
        let field = mutation(mutation_name);
        let arg = field["args"]
            .as_array()
            .expect("mutation should expose args")
            .iter()
            .find(|arg| arg["name"] == arg_name)
            .expect("ID arg should exist");
        assert_eq!(arg["type"]["kind"], "NON_NULL");
        assert_eq!(arg["type"]["ofType"]["name"], "ID");
    };
    for (mutation_name, arg_name) in [
        ("deleteMyPasskey", "id"),
        ("revokeMyOauthApp", "grantId"),
        ("deleteUser", "id"),
        ("resetUserMfa", "id"),
        ("unlinkExternalAccount", "linkedAccountId"),
        ("deleteDelayProfile", "id"),
        ("deleteQualityProfile", "id"),
        ("deletePostProcessingScript", "id"),
        ("togglePostProcessingScript", "id"),
        ("scanTitleLibrary", "titleId"),
        ("cancelLibraryScan", "sessionId"),
        ("ignorePendingImport", "pendingImportId"),
        ("triggerTitleMismatchRecoverySearch", "titleId"),
    ] {
        assert_id_arg(mutation_name, arg_name);
    }

    let scan_library_args = mutation("scanLibrary")["args"]
        .as_array()
        .expect("scanLibrary should expose args");
    assert_eq!(scan_library_args.len(), 1);
    assert_eq!(scan_library_args[0]["name"], "input");
    assert_eq!(scan_library_args[0]["type"]["kind"], "NON_NULL");
    assert_eq!(
        scan_library_args[0]["type"]["ofType"]["name"],
        "ScanLibraryInput"
    );

    let scan_input_fields = body["data"]["scanLibraryInput"]["inputFields"]
        .as_array()
        .expect("ScanLibraryInput should expose input fields");
    let scan_input_field = |name: &str| {
        scan_input_fields
            .iter()
            .find(|field| field["name"] == name)
            .expect("ScanLibraryInput field should exist")
    };
    let library_id = scan_input_field("libraryId");
    assert_eq!(library_id["type"]["kind"], "NON_NULL");
    assert_eq!(library_id["type"]["ofType"]["name"], "ID");
    let import_warmup_session_id = scan_input_field("importWarmupSessionId");
    assert_eq!(import_warmup_session_id["type"]["name"], "ID");

    let rehydrate_args = mutation("rehydrateAllMetadata")["args"]
        .as_array()
        .expect("rehydrateAllMetadata should expose args");
    assert_eq!(rehydrate_args.len(), 1);
    assert_eq!(rehydrate_args[0]["name"], "input");
    assert_eq!(rehydrate_args[0]["type"]["kind"], "NON_NULL");
    assert_eq!(
        rehydrate_args[0]["type"]["ofType"]["name"],
        "RehydrateAllMetadataInput"
    );

    let rehydrate_input_fields = body["data"]["rehydrateAllMetadataInput"]["inputFields"]
        .as_array()
        .expect("RehydrateAllMetadataInput should expose input fields");
    assert_eq!(rehydrate_input_fields.len(), 1);
    assert_eq!(rehydrate_input_fields[0]["name"], "language");
    assert_eq!(rehydrate_input_fields[0]["type"]["kind"], "NON_NULL");
    assert_eq!(
        rehydrate_input_fields[0]["type"]["ofType"]["name"],
        "String"
    );

    for (mutation_name, payload_name) in [
        ("webauthnRegisterComplete", "PasskeySummaryPayload"),
        ("totpEnrollmentComplete", "TotpEnrollmentCompletePayload"),
        ("totpDisable", "TotpStatusPayload"),
        (
            "totpRegenerateRecoveryCodes",
            "TotpEnrollmentCompletePayload",
        ),
        ("deleteMyPasskey", "DeleteMyPasskeyPayload"),
        ("revokeMyOauthApp", "RevokeMyOauthAppPayload"),
        ("deleteUser", "DeleteUserPayload"),
        ("unlinkExternalAccount", "UnlinkExternalAccountPayload"),
        ("clearTitleImageCache", "ClearTitleImageCachePayload"),
        ("completeSetup", "CompleteSetupPayload"),
        (
            "reorderDownloadClientConfigs",
            "ReorderDownloadClientConfigsPayload",
        ),
        ("setTitleRequiredAudio", "SetTitleRequiredAudioPayload"),
        ("rehydrateAllMetadata", "RehydrateAllMetadataPayload"),
        (
            "deletePostProcessingScript",
            "DeletePostProcessingScriptPayload",
        ),
        (
            "triggerTitleMismatchRecoverySearch",
            "TriggerTitleMismatchRecoverySearchPayload",
        ),
    ] {
        assert_eq!(
            mutation(mutation_name)["type"]["ofType"]["name"],
            payload_name
        );
    }

    let payload_fields = |type_alias: &str| -> Vec<&str> {
        body["data"][type_alias]["fields"]
            .as_array()
            .expect("payload should expose fields")
            .iter()
            .filter_map(|field| field["name"].as_str())
            .collect()
    };

    assert_eq!(payload_fields("deleteMyPasskeyPayload"), vec!["id"]);
    assert_eq!(
        payload_fields("revokeMyOauthAppPayload"),
        vec!["grantId", "revoked"]
    );
    assert_eq!(payload_fields("deleteUserPayload"), vec!["id"]);
    assert_eq!(
        payload_fields("unlinkExternalAccountPayload"),
        vec!["linkedAccountId"]
    );
    assert_eq!(
        payload_fields("clearTitleImageCachePayload"),
        vec!["requestedAt"]
    );
    assert_eq!(payload_fields("completeSetupPayload"), vec!["completed"]);
    assert_eq!(
        payload_fields("reorderDownloadClientConfigsPayload"),
        vec!["ids"]
    );
    assert_eq!(
        payload_fields("setTitleRequiredAudioPayload"),
        vec!["titleId", "facet", "languages", "updated"]
    );
    assert_eq!(
        payload_fields("rehydrateAllMetadataPayload"),
        vec!["language", "titlesCleared"]
    );
    assert_eq!(
        payload_fields("deletePostProcessingScriptPayload"),
        vec!["id"]
    );
    assert_eq!(
        payload_fields("triggerTitleMismatchRecoverySearchPayload"),
        vec!["titleId", "queuedCount"]
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_series_movie_search_input_on_search_releases() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          searchReleasesInput: __type(name: "SearchReleasesInput") {
            inputFields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let fields = body["data"]["searchReleasesInput"]["inputFields"]
        .as_array()
        .expect("should have input fields");
    let names: Vec<&str> = fields.iter().filter_map(|f| f["name"].as_str()).collect();

    assert!(names.contains(&"titleId"));
    assert!(names.contains(&"seriesMovieLinkId"));
    assert!(names.contains(&"season"));
    assert!(names.contains(&"episode"));
}

#[tokio::test]
async fn graphql_search_releases_rejects_series_movie_and_episode_inputs_together() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        query SearchReleases($input: SearchReleasesInput!) {
          searchReleases(input: $input) { title }
        }
        "#,
        json!({
            "input": {
                "titleId": "title-1",
                "seriesMovieLinkId": "series-movie-link-1",
                "season": "1",
                "episode": "1"
            }
        }),
    )
    .await;

    let errors = body["errors"].as_array().expect("expected graphql errors");
    let message = errors[0]["message"]
        .as_str()
        .expect("expected graphql error message");
    assert!(message.contains("series movie searches cannot include season or episode"));
}

#[tokio::test]
async fn graphql_introspection_interactive_release_search_uses_payloads() {
    // 0.17.1 hotfix: the streaming interactive release-search job reuses
    // SearchReleasesInput on start, id-anchored poll/cancel, and dedicated
    // payload types with the state / per-indexer-status enums.
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          mutationRoot: __type(name: "MutationRoot") {
            fields {
              name
              args { name type { kind name ofType { kind name } } }
              type { kind name ofType { kind name } }
            }
          }
          queryRoot: __type(name: "QueryRoot") {
            fields {
              name
              args { name type { kind name ofType { kind name } } }
              type { kind name ofType { kind name } }
            }
          }
          stateValue: __type(name: "InteractiveReleaseSearchStateValue") {
            enumValues { name }
          }
          indexerStatusValue: __type(name: "InteractiveReleaseSearchIndexerStatusValue") {
            enumValues { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let field = |root: &str, name: &str| -> Value {
        body["data"][root]["fields"]
            .as_array()
            .expect("root fields")
            .iter()
            .find(|field| field["name"] == name)
            .unwrap_or_else(|| panic!("missing field {name}"))
            .clone()
    };

    let start = field("mutationRoot", "startInteractiveReleaseSearch");
    assert_eq!(start["args"][0]["name"], "input");
    assert_eq!(start["args"][0]["type"]["kind"], "NON_NULL");
    assert_eq!(
        start["args"][0]["type"]["ofType"]["name"],
        "SearchReleasesInput"
    );
    assert_eq!(start["type"]["kind"], "NON_NULL");
    assert_eq!(
        start["type"]["ofType"]["name"],
        "InteractiveReleaseSearchPayload"
    );

    let cancel = field("mutationRoot", "cancelInteractiveReleaseSearch");
    assert_eq!(cancel["args"][0]["name"], "id");
    assert_eq!(cancel["args"][0]["type"]["kind"], "NON_NULL");
    assert_eq!(cancel["args"][0]["type"]["ofType"]["name"], "ID");
    assert_eq!(cancel["type"]["kind"], "NON_NULL");
    assert_eq!(
        cancel["type"]["ofType"]["name"],
        "CancelInteractiveReleaseSearchPayload"
    );

    let poll = field("queryRoot", "interactiveReleaseSearch");
    assert_eq!(poll["args"][0]["name"], "id");
    assert_eq!(poll["args"][0]["type"]["kind"], "NON_NULL");
    assert_eq!(poll["args"][0]["type"]["ofType"]["name"], "ID");
    // Nullable payload: unknown/evicted/foreign job ids resolve to null.
    assert_eq!(poll["type"]["kind"], "OBJECT");
    assert_eq!(poll["type"]["name"], "InteractiveReleaseSearchPayload");

    let enum_values = |key: &str| -> Vec<&str> {
        body["data"][key]["enumValues"]
            .as_array()
            .expect("enum values")
            .iter()
            .filter_map(|value| value["name"].as_str())
            .collect()
    };
    assert_eq!(
        enum_values("stateValue"),
        vec!["RUNNING", "COMPLETED", "CANCELLED"]
    );
    assert_eq!(
        enum_values("indexerStatusValue"),
        vec!["PENDING", "SEARCHING", "COMPLETED", "FAILED", "SKIPPED"]
    );
}

#[tokio::test]
async fn graphql_introspection_exposes_typed_settings_fields() {
    let ctx = TestContext::new().await;
    let body = gql(
        &ctx,
        r#"
        {
          queryRoot: __type(name: "QueryRoot") {
            fields { name }
          }
          mutationRoot: __type(name: "MutationRoot") {
            fields { name }
          }
          subtitleSettings: __type(name: "SubtitleSettingsPayload") {
            fields { name }
          }
          acquisitionSettings: __type(name: "AcquisitionSettingsPayload") {
            fields { name }
          }
          generalSettings: __type(name: "GeneralSettingsPayload") {
            fields { name }
          }
          mediaSettings: __type(name: "MediaSettingsPayload") {
            fields { name }
          }
          librarySettings: __type(name: "LibrarySettingsPayload") {
            fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          librarySettingsInput: __type(name: "LibrarySettingsInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          libraryPaths: __type(name: "LibraryPathsPayload") {
            fields { name }
          }
          serviceSettings: __type(name: "ServiceSettingsPayload") {
            fields { name }
          }
          qualityProfileSettings: __type(name: "QualityProfileSettingsPayload") {
            fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          qualityProfile: __type(name: "QualityProfilePayload") {
            fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          qualityProfileSelection: __type(name: "QualityProfileSelectionPayload") {
            fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          delayProfile: __type(name: "DelayProfilePayload") {
            fields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          qualityProfileCriteriaPayload: __type(name: "QualityProfileCriteriaPayload") {
            fields { name }
          }
          qualityProfileCriteriaInput: __type(name: "QualityProfileCriteriaInput") {
            inputFields { name }
          }
          qualityProfileInput: __type(name: "QualityProfileInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          qualityProfileSelectionInput: __type(name: "QualityProfileSelectionInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          saveQualityProfileSettingsInput: __type(name: "SaveQualityProfileSettingsInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          delayProfileInput: __type(name: "DelayProfileInput") {
            inputFields { name type { kind name ofType { kind name ofType { kind name ofType { kind name } } } } }
          }
          updateSubtitleSettingsInput: __type(name: "UpdateSubtitleSettingsInput") {
            inputFields { name }
          }
          updateGeneralSettingsInput: __type(name: "UpdateGeneralSettingsInput") {
            inputFields { name }
          }
        }
        "#,
        json!({}),
    )
    .await;
    assert_no_errors(&body);

    let query_fields = body["data"]["queryRoot"]["fields"]
        .as_array()
        .expect("QueryRoot should expose fields");
    let query_names: Vec<&str> = query_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(query_names.contains(&"subtitleSettings"));
    assert!(query_names.contains(&"acquisitionSettings"));
    assert!(query_names.contains(&"generalSettings"));
    assert!(query_names.contains(&"mediaSettings"));
    assert!(query_names.contains(&"libraryPaths"));
    assert!(query_names.contains(&"serviceSettings"));
    assert!(query_names.contains(&"qualityProfileSettings"));
    assert!(query_names.contains(&"downloadClientRouting"));
    assert!(query_names.contains(&"indexerRouting"));
    assert!(!query_names.contains(&"convenienceSettings"));
    assert!(!query_names.contains(&"adminSettings"));

    let mutation_fields = body["data"]["mutationRoot"]["fields"]
        .as_array()
        .expect("MutationRoot should expose fields");
    let mutation_names: Vec<&str> = mutation_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(mutation_names.contains(&"updateSubtitleSettings"));
    assert!(mutation_names.contains(&"updateAcquisitionSettings"));
    assert!(mutation_names.contains(&"updateGeneralSettings"));
    assert!(mutation_names.contains(&"updateMediaSettings"));
    assert!(mutation_names.contains(&"updateLibraryPaths"));
    assert!(mutation_names.contains(&"updateServiceSettings"));
    assert!(mutation_names.contains(&"saveQualityProfileSettings"));
    assert!(mutation_names.contains(&"updateDownloadClientRouting"));
    assert!(mutation_names.contains(&"updateIndexerRouting"));
    assert!(!mutation_names.contains(&"updateQualityProfileFacetPersona"));
    assert!(!mutation_names.contains(&"saveAdminSettings"));

    let subtitle_fields = body["data"]["subtitleSettings"]["fields"]
        .as_array()
        .expect("SubtitleSettingsPayload should expose fields");
    let subtitle_names: Vec<&str> = subtitle_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(subtitle_names.contains(&"languages"));
    assert!(!subtitle_names.contains(&"openSubtitlesUsername"));
    assert!(!subtitle_names.contains(&"hasOpenSubtitlesApiKey"));
    assert!(!subtitle_names.contains(&"hasOpenSubtitlesPassword"));

    let subtitle_input_fields = body["data"]["updateSubtitleSettingsInput"]["inputFields"]
        .as_array()
        .expect("UpdateSubtitleSettingsInput should expose input fields");
    let subtitle_input_names: Vec<&str> = subtitle_input_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(!subtitle_input_names.contains(&"openSubtitlesUsername"));
    assert!(!subtitle_input_names.contains(&"openSubtitlesPassword"));
    assert!(!subtitle_input_names.contains(&"openSubtitlesApiKey"));

    let acquisition_fields = body["data"]["acquisitionSettings"]["fields"]
        .as_array()
        .expect("AcquisitionSettingsPayload should expose fields");
    let acquisition_names: Vec<&str> = acquisition_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(acquisition_names.contains(&"pollIntervalSeconds"));
    // The wanted-scheduler cadence knobs (syncIntervalSeconds/batchSize)
    // were replaced by the convergence-cursor knobs.
    assert!(acquisition_names.contains(&"longTailBackfillMaxScopesPerCycle"));
    assert!(acquisition_names.contains(&"longTailReconvergeDays"));
    assert!(!acquisition_names.contains(&"batchSize"));
    assert!(!acquisition_names.contains(&"syncIntervalSeconds"));

    let general_fields = body["data"]["generalSettings"]["fields"]
        .as_array()
        .expect("GeneralSettingsPayload should expose fields");
    let general_names: Vec<&str> = general_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(general_names.contains(&"keepHistoryForever"));
    assert!(general_names.contains(&"historyRetentionDays"));
    assert!(general_names.contains(&"experimentalFeaturesEnabled"));
    assert!(general_names.contains(&"personalizedDiscoveryEnabled"));
    assert!(general_names.contains(&"srrdbFilenameRecoveryEnabled"));

    let media_fields = body["data"]["mediaSettings"]["fields"]
        .as_array()
        .expect("MediaSettingsPayload should expose fields");
    let media_names: Vec<&str> = media_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(media_names.contains(&"libraryPath"));
    assert!(media_names.contains(&"rootFolders"));
    assert!(media_names.contains(&"requiredAudioLanguages"));
    assert!(media_names.contains(&"renameTemplate"));

    let library_fields = body["data"]["libraryPaths"]["fields"]
        .as_array()
        .expect("LibraryPathsPayload should expose fields");
    let library_names: Vec<&str> = library_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(library_names.contains(&"moviePath"));
    assert!(library_names.contains(&"seriesPath"));
    assert!(library_names.contains(&"animePath"));

    let service_fields = body["data"]["serviceSettings"]["fields"]
        .as_array()
        .expect("ServiceSettingsPayload should expose fields");
    let service_names: Vec<&str> = service_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(service_names.contains(&"tlsCertPath"));
    assert!(service_names.contains(&"tlsKeyPath"));

    let settings_output_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["fields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    let settings_input_field = |type_alias: &str, field_name: &str| {
        body["data"][type_alias]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{type_alias} should expose input fields"))
            .iter()
            .find(|field| field["name"] == field_name)
            .unwrap_or_else(|| panic!("{type_alias}.{field_name} should exist"))
            .clone()
    };
    let assert_settings_non_null_id = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["name"], "ID", "{label}");
    };
    let assert_settings_optional_id = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "SCALAR", "{label}");
        assert_eq!(field["type"]["name"], "ID", "{label}");
    };
    let assert_settings_non_null_boolean = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["name"], "Boolean", "{label}");
    };
    let assert_settings_optional_boolean = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "SCALAR", "{label}");
        assert_eq!(field["type"]["name"], "Boolean", "{label}");
    };
    let assert_settings_non_null_id_list = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["kind"], "LIST", "{label}");
        assert_eq!(
            field["type"]["ofType"]["ofType"]["kind"], "NON_NULL",
            "{label}"
        );
        assert_eq!(
            field["type"]["ofType"]["ofType"]["ofType"]["name"], "ID",
            "{label}"
        );
    };
    let assert_settings_optional_id_list = |field: Value, label: &str| {
        assert_eq!(field["type"]["kind"], "LIST", "{label}");
        assert_eq!(field["type"]["ofType"]["kind"], "NON_NULL", "{label}");
        assert_eq!(field["type"]["ofType"]["ofType"]["name"], "ID", "{label}");
    };

    assert_settings_non_null_id(
        settings_output_field("delayProfile", "id"),
        "DelayProfile.id",
    );
    assert_settings_non_null_id(
        settings_input_field("delayProfileInput", "id"),
        "DelayProfileInput.id",
    );
    for field_name in ["enableUsenet", "enableTorrent", "bypassIfHighestQuality"] {
        assert_settings_optional_boolean(
            settings_input_field("delayProfileInput", field_name),
            &format!("DelayProfileInput.{field_name}"),
        );
        assert_settings_non_null_boolean(
            settings_output_field("delayProfile", field_name),
            &format!("DelayProfilePayload.{field_name}"),
        );
    }
    assert_settings_non_null_id(
        settings_output_field("qualityProfile", "id"),
        "QualityProfilePayload.id",
    );
    assert_settings_non_null_id(
        settings_input_field("qualityProfileInput", "id"),
        "QualityProfileInput.id",
    );
    assert_settings_non_null_id(
        settings_output_field("qualityProfileSettings", "globalProfileId"),
        "QualityProfileSettingsPayload.globalProfileId",
    );
    assert_settings_optional_id(
        settings_output_field("qualityProfileSelection", "overrideProfileId"),
        "QualityProfileSelectionPayload.overrideProfileId",
    );
    assert_settings_non_null_id(
        settings_output_field("qualityProfileSelection", "effectiveProfileId"),
        "QualityProfileSelectionPayload.effectiveProfileId",
    );
    assert_settings_optional_id(
        settings_input_field("qualityProfileSelectionInput", "profileId"),
        "QualityProfileSelectionInput.profileId",
    );
    assert_settings_optional_id(
        settings_input_field("saveQualityProfileSettingsInput", "globalProfileId"),
        "SaveQualityProfileSettingsInput.globalProfileId",
    );
    assert_settings_optional_id(
        settings_output_field("librarySettings", "qualityProfileIdOverride"),
        "LibrarySettingsPayload.qualityProfileIdOverride",
    );
    assert_settings_non_null_id(
        settings_output_field("librarySettings", "qualityProfileId"),
        "LibrarySettingsPayload.qualityProfileId",
    );
    assert_settings_optional_id_list(
        settings_output_field("librarySettings", "requestQualityProfileIdsOverride"),
        "LibrarySettingsPayload.requestQualityProfileIdsOverride",
    );
    assert_settings_non_null_id_list(
        settings_output_field("librarySettings", "requestQualityProfileIds"),
        "LibrarySettingsPayload.requestQualityProfileIds",
    );
    assert_settings_non_null_id(
        settings_output_field("librarySettings", "requestQualityProfileDefaultId"),
        "LibrarySettingsPayload.requestQualityProfileDefaultId",
    );
    assert_settings_optional_id(
        settings_input_field("librarySettingsInput", "qualityProfileId"),
        "LibrarySettingsInput.qualityProfileId",
    );
    assert_settings_optional_id_list(
        settings_input_field("librarySettingsInput", "requestQualityProfileIds"),
        "LibrarySettingsInput.requestQualityProfileIds",
    );

    let quality_profile_settings_fields = body["data"]["qualityProfileSettings"]["fields"]
        .as_array()
        .expect("QualityProfileSettingsPayload should expose fields");
    let quality_profile_settings_names: Vec<&str> = quality_profile_settings_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(quality_profile_settings_names.contains(&"globalScoringPersona"));
    assert!(quality_profile_settings_names.contains(&"categoryPersonaSelections"));

    let criteria_payload_fields = body["data"]["qualityProfileCriteriaPayload"]["fields"]
        .as_array()
        .expect("QualityProfileCriteriaPayload should expose fields");
    let criteria_payload_names: Vec<&str> = criteria_payload_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(!criteria_payload_names.contains(&"requiredAudioLanguages"));
    assert!(!criteria_payload_names.contains(&"scoringPersona"));
    assert!(!criteria_payload_names.contains(&"facetPersonaOverrides"));
    assert!(!criteria_payload_names.contains(&"atmosPreferred"));

    let criteria_input_fields = body["data"]["qualityProfileCriteriaInput"]["inputFields"]
        .as_array()
        .expect("QualityProfileCriteriaInput should expose inputFields");
    let criteria_input_names: Vec<&str> = criteria_input_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(!criteria_input_names.contains(&"requiredAudioLanguages"));
    assert!(!criteria_input_names.contains(&"scoringPersona"));
    assert!(!criteria_input_names.contains(&"facetPersonaOverrides"));
    assert!(!criteria_input_names.contains(&"atmosPreferred"));

    let general_input_fields = body["data"]["updateGeneralSettingsInput"]["inputFields"]
        .as_array()
        .expect("UpdateGeneralSettingsInput should expose inputFields");
    let general_input_names: Vec<&str> = general_input_fields
        .iter()
        .filter_map(|field| field["name"].as_str())
        .collect();
    assert!(general_input_names.contains(&"keepHistoryForever"));
    assert!(general_input_names.contains(&"historyRetentionDays"));
    assert!(general_input_names.contains(&"experimentalFeaturesEnabled"));
    assert!(general_input_names.contains(&"personalizedDiscoveryEnabled"));
    assert!(general_input_names.contains(&"srrdbFilenameRecoveryEnabled"));
}
