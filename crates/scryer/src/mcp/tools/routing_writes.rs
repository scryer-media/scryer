use super::rule_writes::{PRIORITY_MAX, PRIORITY_MIN};
use super::{
    Args, Effect, Kind, Param, SCOPE_PARAM, ToolContext, ToolFuture, ToolSpec, mutation_result,
};
use serde_json::json;

/// Replaces one scope's indexer routing.
pub(super) const INDEXER_OPERATION: &str = include_str!(
    "../../../../../skills/scryer-configure-acquisition/operations/UpdateIndexerRouting.graphql"
);

/// Replaces one scope's download-client routing.
pub(super) const DOWNLOAD_CLIENT_OPERATION: &str = include_str!(
    "../../../../../skills/scryer-configure-acquisition/operations/UpdateDownloadClientRouting.graphql"
);

/// Sets the download-client order.
pub(super) const REORDER_OPERATION: &str = include_str!(
    "../../../../../skills/scryer-configure-acquisition/operations/ReorderDownloadClients.graphql"
);

/// Pins an indexer's grabs to one download client, or clears the pin.
pub(super) const MAPPING_OPERATION: &str = include_str!(
    "../../../../../skills/scryer-configure-acquisition/operations/SetIndexerDownloadClientMapping.graphql"
);

/// Indexers or download clients one call names at most.
const MAX_ENTRIES: usize = 200;

const fn id_field(name: &'static str, description: &'static str) -> Param {
    Param {
        name,
        description,
        required: true,
        kind: Kind::Str { max_chars: 100 },
    }
}

const fn flag(name: &'static str, description: &'static str) -> Param {
    Param {
        name,
        description,
        required: true,
        kind: Kind::Bool { default: None },
    }
}

const fn label(name: &'static str, description: &'static str, max_chars: usize) -> Param {
    Param {
        name,
        description,
        required: false,
        kind: Kind::Str { max_chars },
    }
}

const INDEXER_ENTRY_FIELDS: &[Param] = &[
    id_field("indexerId", "The indexer's id, from list_indexers."),
    flag("enabled", "Whether the indexer is searched for this scope."),
    Param {
        name: "categories",
        description: "Indexer categories to request, as get_indexer_routing returns them.",
        required: true,
        kind: Kind::StrSeq {
            min_items: 0,
            max_items: 100,
            max_chars: 50,
        },
    },
    Param {
        name: "priority",
        description: "Relative indexer priority.",
        required: true,
        kind: Kind::Int {
            default: None,
            min: PRIORITY_MIN,
            max: PRIORITY_MAX,
        },
    },
];

const DOWNLOAD_CLIENT_ENTRY_FIELDS: &[Param] = &[
    id_field(
        "clientId",
        "The download client's id, from list_download_clients.",
    ),
    flag("enabled", "Whether the client takes grabs for this scope."),
    label(
        "category",
        "Category sent to the client; omit for none.",
        200,
    ),
    label(
        "recentQueuePriority",
        "Priority label for recent queue items; omit for none.",
        50,
    ),
    label(
        "olderQueuePriority",
        "Priority label for older queue items; omit for none.",
        50,
    ),
    flag(
        "removeCompleted",
        "Whether Scryer removes completed items from the client after import.",
    ),
    flag(
        "removeFailed",
        "Whether Scryer removes failed items from the client.",
    ),
    label(
        "seedingProfileId",
        "Seeding profile for torrents, from list_seeding_profiles; omit for none.",
        100,
    ),
];

pub(super) static INDEXER_TOOL: ToolSpec = ToolSpec {
    name: "update_indexer_routing",
    title: "Update indexer routing",
    description: "Replace one content scope's indexer routing through the updateIndexerRouting \
mutation. The entries given become the whole scope, so send every entry to keep, as \
get_indexer_routing returns them. Changing or removing an indexer's entry resets what Scryer has \
learned about searching that indexer. Returns the saved entries.",
    params: &[
        SCOPE_PARAM,
        Param {
            name: "entries",
            description: "Every routing entry of the scope.",
            required: true,
            kind: Kind::ObjectList {
                fields: INDEXER_ENTRY_FIELDS,
                min_items: 0,
                max_items: MAX_ENTRIES,
            },
        },
    ],
    effect: Effect::Write {
        destructive: true,
        idempotent: true,
    },
    run: run_indexer,
};

pub(super) static DOWNLOAD_CLIENT_TOOL: ToolSpec = ToolSpec {
    name: "update_download_client_routing",
    title: "Update download client routing",
    description: "Replace one content scope's download-client routing through the \
updateDownloadClientRouting mutation. The entries given become the whole scope, highest \
priority first, so send every entry to keep, as get_download_client_routing returns them. \
removeCompleted and removeFailed make Scryer remove items from the client. Returns the saved \
entries.",
    params: &[
        SCOPE_PARAM,
        Param {
            name: "entries",
            description: "Every routing entry of the scope, highest priority first.",
            required: true,
            kind: Kind::ObjectList {
                fields: DOWNLOAD_CLIENT_ENTRY_FIELDS,
                min_items: 0,
                max_items: MAX_ENTRIES,
            },
        },
    ],
    effect: Effect::Write {
        destructive: true,
        idempotent: true,
    },
    run: run_download_client,
};

pub(super) static REORDER_TOOL: ToolSpec = ToolSpec {
    name: "reorder_download_clients",
    title: "Reorder download clients",
    description: "Set the download-client list order through the reorderDownloadClientConfigs \
mutation, first highest. Send every client id from list_download_clients: an unknown id is \
ignored, and a client left out keeps its old priority number, which can then tie with or sit \
among the new positions. The result echoes the ids sent, not the saved order, so confirm with \
list_download_clients.",
    params: &[Param {
        name: "ids",
        description: "Download client ids in the new order, from list_download_clients.",
        required: true,
        kind: Kind::StrList {
            min_items: 1,
            max_items: MAX_ENTRIES,
            max_chars: 100,
        },
    }],
    effect: Effect::Write {
        destructive: true,
        idempotent: true,
    },
    run: run_reorder,
};

pub(super) static MAPPING_TOOL: ToolSpec = ToolSpec {
    name: "set_indexer_download_client_mapping",
    title: "Pin an indexer to a download client",
    description: "Send an indexer's grabs to one download client, or clear that pin, through \
the setIndexerDownloadClientMapping mutation. Omit download_client_id to clear it. Returns the \
indexer's id, name and download client id; list_indexers shows it.",
    params: &[
        id_field("indexer_id", "The indexer's id, from list_indexers."),
        label(
            "download_client_id",
            "The download client, from list_download_clients; omit to clear the pin.",
            100,
        ),
    ],
    effect: Effect::Write {
        destructive: true,
        idempotent: true,
    },
    run: run_mapping,
};

fn routing_variables(args: &Args) -> serde_json::Value {
    json!({"input": {"scope": args.value("scope"), "entries": args.value("entries")}})
}

fn run_indexer(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        mutation_result(
            &context,
            INDEXER_OPERATION,
            routing_variables(&args),
            "updateIndexerRouting",
        )
        .await
    })
}

fn run_download_client(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        mutation_result(
            &context,
            DOWNLOAD_CLIENT_OPERATION,
            routing_variables(&args),
            "updateDownloadClientRouting",
        )
        .await
    })
}

fn run_reorder(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        mutation_result(
            &context,
            REORDER_OPERATION,
            json!({"input": {"ids": args.value("ids")}}),
            "reorderDownloadClientConfigs",
        )
        .await
    })
}

fn run_mapping(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        mutation_result(
            &context,
            MAPPING_OPERATION,
            json!({"input": {
                "indexerId": args.value("indexer_id"),
                "downloadClientId": args.value("download_client_id"),
            }}),
            "setIndexerDownloadClientMapping",
        )
        .await
    })
}
