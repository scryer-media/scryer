use super::{
    Args, Effect, OFFSET_PARAM, SCOPE_PARAM, ToolContext, ToolFuture, ToolSpec, field, graphql,
    limit_param, list_result, offset_page,
};
use serde_json::{Map, Value, json};

/// Delay profiles.
pub(super) const DELAY_PROFILES_OPERATION: &str = include_str!(
    "../../../../../skills/scryer-configure-acquisition/operations/DelayProfiles.graphql"
);

/// Quality profiles, the global default and the per-scope selections.
pub(super) const QUALITY_PROFILES_OPERATION: &str = include_str!(
    "../../../../../skills/scryer-configure-acquisition/operations/QualityProfileSettings.graphql"
);

/// Indexer routing for one scope.
pub(super) const INDEXER_ROUTING_OPERATION: &str = include_str!(
    "../../../../../skills/scryer-configure-acquisition/operations/IndexerRouting.graphql"
);

/// Download-client routing for one scope.
pub(super) const DOWNLOAD_CLIENT_ROUTING_OPERATION: &str = include_str!(
    "../../../../../skills/scryer-configure-acquisition/operations/DownloadClientRouting.graphql"
);

/// Seeding profiles.
pub(super) const SEEDING_PROFILES_OPERATION: &str = include_str!(
    "../../../../../skills/scryer-configure-acquisition/operations/SeedingProfiles.graphql"
);

pub(super) static DELAY_PROFILES_TOOL: ToolSpec = ToolSpec {
    name: "list_delay_profiles",
    title: "Delay profiles",
    description: "List delay profiles with every field upsert_delay_profile takes: delays per \
protocol, protocol switches and preference, minimum age, bypass settings, facets, tags, \
libraries, priority and enabled.",
    params: &[limit_param("Profiles to return.", 50), OFFSET_PARAM],
    effect: Effect::Read,
    run: run_delay_profiles,
};

pub(super) static QUALITY_PROFILES_TOOL: ToolSpec = ToolSpec {
    name: "get_quality_profile_settings",
    title: "Quality profile settings",
    description: "The global quality profile and scoring persona, the per-scope selections, \
and a page of quality profiles with their full criteria, in the shape \
save_quality_profile_settings takes.",
    params: &[limit_param("Profiles to return.", 10), OFFSET_PARAM],
    effect: Effect::Read,
    run: run_quality_profiles,
};

pub(super) static INDEXER_ROUTING_TOOL: ToolSpec = ToolSpec {
    name: "get_indexer_routing",
    title: "Indexer routing",
    description: "The indexer routing entries of one content scope: indexer id, enabled, \
categories and priority. update_indexer_routing replaces the whole scope, so read every page \
first.",
    params: &[
        SCOPE_PARAM,
        limit_param("Entries to return.", 100),
        OFFSET_PARAM,
    ],
    effect: Effect::Read,
    run: run_indexer_routing,
};

pub(super) static DOWNLOAD_CLIENT_ROUTING_TOOL: ToolSpec = ToolSpec {
    name: "get_download_client_routing",
    title: "Download client routing",
    description: "The download-client routing entries of one content scope, highest priority \
first: client id, enabled, category, queue priorities, whether completed and failed items are \
removed from the client, and seeding profile. update_download_client_routing replaces the \
whole scope, so read every page first.",
    params: &[
        SCOPE_PARAM,
        limit_param("Entries to return.", 100),
        OFFSET_PARAM,
    ],
    effect: Effect::Read,
    run: run_download_client_routing,
};

pub(super) static SEEDING_PROFILES_TOOL: ToolSpec = ToolSpec {
    name: "list_seeding_profiles",
    title: "Seeding profiles",
    description: "List the seeding profiles torrent routing entries can name: id, name, ratio \
goal, seed time and minimum seeders.",
    params: &[limit_param("Profiles to return.", 50), OFFSET_PARAM],
    effect: Effect::Read,
    run: run_seeding_profiles,
};

/// Runs `document` and pages the list at `path`, with `head` built from the
/// whole list and the data.
async fn paged(
    context: &ToolContext,
    args: &Args,
    document: &'static str,
    variables: Value,
    path: &[&str],
    head: impl FnOnce(&Value, &[Value]) -> Map<String, Value>,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    let data = match graphql(context, document, variables).await {
        Ok(data) => data,
        Err(error) => return Ok(error),
    };
    let rows = path
        .iter()
        .fold(&data, |value, name| field(value, name))
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut head = head(&data, &rows);
    head.entry("total").or_insert(json!(rows.len()));
    let total = rows.len();
    let (items, next) = offset_page(rows, total, args.count("offset"), args.count("limit"));
    Ok(list_result(head, items, "next_offset", next))
}

fn enabled_count(rows: &[Value], name: &str) -> Value {
    json!(
        rows.iter()
            .filter(|row| field(row, name).as_bool() == Some(true))
            .count()
    )
}

fn run_delay_profiles(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        paged(
            &context,
            &args,
            DELAY_PROFILES_OPERATION,
            Value::Null,
            &["delayProfiles"],
            |_, rows| {
                let mut head = Map::new();
                head.insert("total".into(), json!(rows.len()));
                head.insert("enabled".into(), enabled_count(rows, "enabled"));
                head
            },
        )
        .await
    })
}

fn run_quality_profiles(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        paged(
            &context,
            &args,
            QUALITY_PROFILES_OPERATION,
            Value::Null,
            &["qualityProfileSettings", "profiles"],
            |data, _| {
                let settings = field(data, "qualityProfileSettings");
                [
                    "globalProfileId",
                    "globalScoringPersona",
                    "categorySelections",
                    "categoryPersonaSelections",
                ]
                .into_iter()
                .map(|name| (name.to_string(), field(settings, name).clone()))
                .collect()
            },
        )
        .await
    })
}

fn run_indexer_routing(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let scope = args.value("scope");
        paged(
            &context,
            &args,
            INDEXER_ROUTING_OPERATION,
            json!({"scope": scope}),
            &["indexerRouting"],
            |_, rows| {
                let mut head = Map::new();
                head.insert("scope".into(), scope.clone());
                head.insert("total".into(), json!(rows.len()));
                head.insert("enabled".into(), enabled_count(rows, "enabled"));
                head
            },
        )
        .await
    })
}

fn run_download_client_routing(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let scope = args.value("scope");
        paged(
            &context,
            &args,
            DOWNLOAD_CLIENT_ROUTING_OPERATION,
            json!({"scope": scope}),
            &["downloadClientRouting"],
            |_, rows| {
                let mut head = Map::new();
                head.insert("scope".into(), scope.clone());
                head.insert("total".into(), json!(rows.len()));
                head.insert("enabled".into(), enabled_count(rows, "enabled"));
                head
            },
        )
        .await
    })
}

fn run_seeding_profiles(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        paged(
            &context,
            &args,
            SEEDING_PROFILES_OPERATION,
            Value::Null,
            &["seedingProfiles"],
            |_, rows| {
                let mut head = Map::new();
                head.insert("total".into(), json!(rows.len()));
                head
            },
        )
        .await
    })
}
