use super::rule_writes::{PRIORITY_MAX, PRIORITY_MIN};
use super::{
    Args, Effect, Kind, Param, ToolContext, ToolFuture, ToolSpec, graphql_enum_names,
    mutation_result,
};
use serde_json::json;

/// Creates or replaces the delay profile with the given id.
pub(super) const OPERATION: &str = include_str!(
    "../../../../../skills/scryer-configure-acquisition/operations/UpsertDelayProfile.graphql"
);

/// A year of minutes: the longest delay or minimum age these tools set.
const MAX_MINUTES: i64 = 525_600;

fn protocols() -> Vec<&'static str> {
    graphql_enum_names::<scryer_interface::types::DelayProfilePreferredProtocolValue>()
}

pub(super) fn media_facets() -> Vec<&'static str> {
    graphql_enum_names::<scryer_interface::types::MediaFacetValue>()
}

const fn minutes(name: &'static str, description: &'static str) -> Param {
    Param {
        name,
        description,
        required: true,
        kind: Kind::Int {
            default: None,
            min: 0,
            max: MAX_MINUTES,
        },
    }
}

const PROFILE_FIELDS: &[Param] = &[
    Param {
        name: "id",
        description: "The profile's id: an existing one to replace it, or a new one to add a \
profile.",
        required: true,
        kind: Kind::Str { max_chars: 100 },
    },
    Param {
        name: "name",
        description: "Display name.",
        required: true,
        kind: Kind::Str { max_chars: 200 },
    },
    minutes("usenetDelayMinutes", "Usenet delay in minutes."),
    minutes("torrentDelayMinutes", "Torrent delay in minutes."),
    Param {
        name: "enableUsenet",
        description: "Whether Usenet releases are eligible (default true).",
        required: false,
        kind: Kind::Bool {
            default: Some(true),
        },
    },
    Param {
        name: "enableTorrent",
        description: "Whether torrent releases are eligible (default true).",
        required: false,
        kind: Kind::Bool {
            default: Some(true),
        },
    },
    Param {
        name: "preferredProtocol",
        description: "The protocol preferred when both qualify.",
        required: true,
        kind: Kind::Enum {
            values: protocols,
            default: None,
        },
    },
    minutes("minAgeMinutes", "Minimum release age in minutes."),
    Param {
        name: "bypassScoreThreshold",
        description: "A score at or above which the delay is skipped; omit for none.",
        required: false,
        kind: Kind::Int {
            default: None,
            min: -1_000_000,
            max: 1_000_000,
        },
    },
    Param {
        name: "bypassIfHighestQuality",
        description: "Whether the highest-quality release skips its delay (default false).",
        required: false,
        kind: Kind::Bool {
            default: Some(false),
        },
    },
    Param {
        name: "appliesToFacets",
        description: "The facets the profile applies to: MOVIE, SERIES, ANIME.",
        required: true,
        kind: Kind::EnumList {
            values: media_facets,
            min_items: 0,
            max_items: 3,
        },
    },
    Param {
        name: "tags",
        description: "Title tags the profile is limited to; empty for none.",
        required: true,
        kind: Kind::StrList {
            min_items: 0,
            max_items: 50,
            max_chars: 100,
        },
    },
    Param {
        name: "libraryIds",
        description: "Libraries the profile is limited to; empty for every library. Omit to \
keep an existing profile's libraries.",
        required: false,
        kind: Kind::StrList {
            min_items: 0,
            max_items: 100,
            max_chars: 100,
        },
    },
    Param {
        name: "priority",
        description: "Relative priority among delay profiles.",
        required: true,
        kind: Kind::Int {
            default: None,
            min: PRIORITY_MIN,
            max: PRIORITY_MAX,
        },
    },
    Param {
        name: "enabled",
        description: "Whether the profile is active.",
        required: true,
        kind: Kind::Bool { default: None },
    },
];

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "upsert_delay_profile",
    title: "Save a delay profile",
    description: "Save one delay profile through the upsertDelayProfile mutation: a profile \
with an existing id is replaced whole, a new id adds one. Send every field, as \
list_delay_profiles returns them. Returns the saved profile.",
    params: &[Param {
        name: "profile",
        description: "The whole profile.",
        required: true,
        kind: Kind::Object {
            fields: PROFILE_FIELDS,
        },
    }],
    effect: Effect::Write {
        destructive: true,
        idempotent: true,
    },
    run,
};

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        mutation_result(
            &context,
            OPERATION,
            json!({"input": args.value("profile")}),
            "upsertDelayProfile",
        )
        .await
    })
}
