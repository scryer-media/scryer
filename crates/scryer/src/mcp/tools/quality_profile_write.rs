use super::{
    Args, Effect, Kind, Param, ToolContext, ToolFuture, ToolSpec, content_scopes,
    graphql_enum_names, mutation_result,
};
use serde_json::{Map, Value, json};

/// Saves quality profiles and selections, merged by id.
pub(super) const OPERATION: &str = include_str!(
    "../../../../../skills/scryer-configure-acquisition/operations/SaveQualityProfileSettings.graphql"
);

/// Profiles one call saves at most.
const MAX_PROFILES: usize = 25;

/// Identifiers in one tier, source or codec list at most.
const MAX_LIST_ITEMS: usize = 50;

/// The largest score magnitude these tools accept.
const MAX_SCORE: i64 = 1_000_000;

fn personas() -> Vec<&'static str> {
    graphql_enum_names::<scryer_interface::types::ScoringPersonaValue>()
}

fn decision_gates() -> Vec<&'static str> {
    graphql_enum_names::<scryer_interface::types::QualityDecisionGateValue>()
}

const fn ids(name: &'static str, description: &'static str) -> Param {
    Param {
        name,
        description,
        required: true,
        kind: Kind::StrList {
            min_items: 0,
            max_items: MAX_LIST_ITEMS,
            max_chars: 100,
        },
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

const fn optional_flag(name: &'static str, description: &'static str) -> Param {
    Param {
        name,
        description,
        required: false,
        kind: Kind::Bool { default: None },
    }
}

const fn optional_int(name: &'static str, description: &'static str, min: i64, max: i64) -> Param {
    Param {
        name,
        description,
        required: false,
        kind: Kind::Int {
            default: None,
            min,
            max,
        },
    }
}

const fn optional_str(name: &'static str, description: &'static str) -> Param {
    Param {
        name,
        description,
        required: false,
        kind: Kind::Str { max_chars: 100 },
    }
}

const SCORING_OVERRIDE_FIELDS: &[Param] = &[
    optional_flag(
        "allowX265Non4K",
        "Whether non-4K x265 releases are allowed.",
    ),
    optional_flag(
        "blockDvWithoutFallback",
        "Whether Dolby Vision without a fallback is blocked.",
    ),
    optional_flag(
        "preferCompactEncodes",
        "Whether compact encodes are preferred.",
    ),
    optional_flag(
        "preferLosslessAudio",
        "Whether lossless audio is preferred.",
    ),
    optional_flag("blockUpscaled", "Whether upscaled releases are blocked."),
];

const CRITERIA_FIELDS: &[Param] = &[
    ids("qualityTiers", "Ordered quality tiers, best first."),
    optional_str(
        "archivalQuality",
        "Archival quality tier; omitting it clears the saved one.",
    ),
    flag(
        "allowUnknownQuality",
        "Whether releases of unknown quality are allowed.",
    ),
    ids("sourceAllowlist", "Allowed release sources."),
    ids("sourceBlocklist", "Blocked release sources."),
    ids("videoCodecAllowlist", "Allowed video codecs."),
    ids("videoCodecBlocklist", "Blocked video codecs."),
    ids("audioCodecAllowlist", "Allowed audio codecs."),
    ids("audioCodecBlocklist", "Blocked audio codecs."),
    flag("dolbyVisionAllowed", "Whether Dolby Vision is allowed."),
    flag("detectedHdrAllowed", "Whether detected HDR is allowed."),
    flag("preferRemux", "Whether remux releases are preferred."),
    flag("allowBdDisk", "Whether Blu-ray disk releases are allowed."),
    flag(
        "allowUpgrades",
        "Whether a better release replaces the library file.",
    ),
    optional_flag(
        "ignoreProperRepack",
        "Whether scores decide without the PROPER/REPACK preference; omit to keep the saved \
value. When decisionPipeline is given, whether it lists REVISION decides this instead.",
    ),
    Param {
        name: "scoringOverrides",
        description: "Additional scoring preferences, saved as sent: an omitted field clears \
the saved one.",
        required: true,
        kind: Kind::Object {
            fields: SCORING_OVERRIDE_FIELDS,
        },
    },
    optional_str(
        "cutoffTier",
        "Cutoff quality tier; omitting it clears the saved one.",
    ),
    optional_int(
        "minScoreToGrab",
        "Minimum score to grab a release; omitting it clears the saved one.",
        -MAX_SCORE,
        MAX_SCORE,
    ),
    optional_int(
        "cutoffScore",
        "Score past which same-tier upgrades stop; omitting it clears the saved one.",
        -MAX_SCORE,
        MAX_SCORE,
    ),
    optional_int(
        "minUpgradeScoreDelta",
        "Score a same-tier candidate must beat the current file by; omit to keep the saved \
value.",
        0,
        MAX_SCORE,
    ),
    optional_int(
        "minMissingEpisodesForPack",
        "Missing wanted episodes before an automatic search takes a pack; omit to keep the \
saved value.",
        1,
        10_000,
    ),
    optional_int(
        "minMissingPercentForSeriesPack",
        "Percent of a multi-season pack's aired episodes that must be missing; omit to keep \
the saved value.",
        0,
        100,
    ),
    Param {
        name: "decisionPipeline",
        description: "Decision order, each step at most once; omit to keep the saved order, \
empty ranks on score alone.",
        required: false,
        kind: Kind::EnumList {
            values: decision_gates,
            min_items: 0,
            max_items: 16,
        },
    },
];

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
    Param {
        name: "criteria",
        description: "The profile's criteria, as get_quality_profile_settings returns them.",
        required: true,
        kind: Kind::Object {
            fields: CRITERIA_FIELDS,
        },
    },
];

const SCOPE_FIELD: Param = Param {
    name: "scope",
    description: "MOVIE, SERIES or ANIME.",
    required: true,
    kind: Kind::Enum {
        values: content_scopes,
        default: None,
    },
};

const INHERIT_FIELD: Param = flag("inheritGlobal", "Whether the scope uses the global value.");

const SELECTION_FIELDS: &[Param] = &[
    SCOPE_FIELD,
    Param {
        name: "profileId",
        description: "The scope's profile; omit when inheriting.",
        required: false,
        kind: Kind::Str { max_chars: 100 },
    },
    INHERIT_FIELD,
];

const PERSONA_SELECTION_FIELDS: &[Param] = &[
    SCOPE_FIELD,
    Param {
        name: "persona",
        description: "The scope's scoring persona; omit when inheriting.",
        required: false,
        kind: Kind::Enum {
            values: personas,
            default: None,
        },
    },
    INHERIT_FIELD,
];

pub(super) static TOOL: ToolSpec = ToolSpec {
    name: "save_quality_profile_settings",
    title: "Save quality profiles",
    description: "Save quality profiles and selections through the saveQualityProfileSettings \
mutation, merged by id: each profile given replaces the saved profile with that id whole or \
adds a new one, and profiles not given are kept. global_profile_id, global_scoring_persona and \
each selection list change only when given; an empty or omitted list leaves the saved \
selections. A selection's profileId is the read's overrideProfileId, and persona its \
overridePersona. Returns the saved global values, selections and profile ids and names.",
    params: &[
        Param {
            name: "profiles",
            description: "Profiles to save, each whole, at most 25.",
            required: false,
            kind: Kind::ObjectList {
                fields: PROFILE_FIELDS,
                min_items: 0,
                max_items: MAX_PROFILES,
            },
        },
        Param {
            name: "global_profile_id",
            description: "The global default profile; omit to keep the saved one.",
            required: false,
            kind: Kind::Str { max_chars: 100 },
        },
        Param {
            name: "global_scoring_persona",
            description: "The global scoring persona; omit to keep the saved one.",
            required: false,
            kind: Kind::Enum {
                values: personas,
                default: None,
            },
        },
        Param {
            name: "category_selections",
            description: "Per-scope profile selections.",
            required: false,
            kind: Kind::ObjectList {
                fields: SELECTION_FIELDS,
                min_items: 0,
                max_items: 3,
            },
        },
        Param {
            name: "category_persona_selections",
            description: "Per-scope scoring persona selections.",
            required: false,
            kind: Kind::ObjectList {
                fields: PERSONA_SELECTION_FIELDS,
                min_items: 0,
                max_items: 3,
            },
        },
    ],
    effect: Effect::Write {
        destructive: true,
        idempotent: true,
    },
    run,
};

fn list_or_empty(args: &Args, name: &str) -> Value {
    match args.value(name) {
        Value::Null => json!([]),
        value => value,
    }
}

fn run(context: ToolContext, args: Args) -> ToolFuture {
    Box::pin(async move {
        let mut input = Map::new();
        input.insert("profiles".into(), list_or_empty(&args, "profiles"));
        input.insert("globalProfileId".into(), args.value("global_profile_id"));
        input.insert(
            "globalScoringPersona".into(),
            args.value("global_scoring_persona"),
        );
        input.insert(
            "categorySelections".into(),
            list_or_empty(&args, "category_selections"),
        );
        input.insert(
            "categoryPersonaSelections".into(),
            list_or_empty(&args, "category_persona_selections"),
        );
        // A replacing save removes every profile not listed; this tool only
        // merges.
        input.insert("replaceExisting".into(), json!(false));
        mutation_result(
            &context,
            OPERATION,
            json!({"input": input}),
            "saveQualityProfileSettings",
        )
        .await
    })
}
