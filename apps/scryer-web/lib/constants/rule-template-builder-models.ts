import {
  RULE_BUILDER_MODEL_VERSION,
  type RuleBuilderCondition,
  type RuleBuilderJoin,
  type RuleBuilderModel,
  type RuleBuilderSchema,
} from "../utils/rule-builder.ts";

/// The simple builder's model of each starter template the builder can
/// express, by rule family and then by template id. Picking one of these
/// templates opens it in the builder already filled in. The builder writes a
/// template's source from its model, and a test holds every model to writing
/// its template's source byte for byte, so the two cannot drift; a template
/// with no model here opens as Rego.

function model(
  family: string,
  conditions: [field: string, operator: string, value?: string][],
  outputs: RuleBuilderModel["outputs"],
  join: RuleBuilderJoin = "ALL",
): RuleBuilderModel {
  return {
    version: RULE_BUILDER_MODEL_VERSION,
    family,
    join,
    conditions: conditions.map(
      ([field, operator, value = ""]): RuleBuilderCondition => ({
        field,
        operator,
        value,
      }),
    ),
    outputs,
  };
}

const facts = (field: string) => `input.facts.${field}`;

/// The model maker for one family, named as the server's builder schema
/// names it.
const family =
  (name: string) =>
  (
    conditions: Parameters<typeof model>[1],
    outputs: RuleBuilderModel["outputs"],
  ): RuleBuilderModel =>
    model(name, conditions, outputs);

/// A lifecycle template's model: shared conditions, then one action row per
/// output, all on `subject`, each with `rowConditions` of its own.
const lifecycle = (
  conditions: Parameters<typeof model>[1],
  outputs: RuleBuilderModel["outputs"],
  subject = "TITLE",
  rowConditions: Parameters<typeof model>[1] = [],
): RuleBuilderModel => ({
  ...model("lifecycle", conditions, outputs),
  actions: Object.keys(outputs).map((output) => ({
    subject,
    output,
    join: "ALL",
    conditions: model("lifecycle", rowConditions, {}).conditions,
  })),
});
const titles = (
  conditions: Parameters<typeof model>[1],
  outputs: RuleBuilderModel["outputs"],
) => lifecycle(conditions, outputs);
const request = family("request");
const scoring = family("scoring");

export const RULE_TEMPLATE_BUILDER_MODELS: Readonly<
  Record<string, Readonly<Record<string, RuleBuilderModel>>>
> = {
  lifecycle: {
    "dead-wanted-entries": titles(
      [
        [facts("monitored"), "IS_TRUE"],
        [facts("has_file"), "IS_FALSE"],
      ],
      { UNMONITOR: true },
    ),
    "library-aging": titles(
      [
        [facts("has_file"), "IS_TRUE"],
        [facts("tags"), "NOT_CONTAINS", "keep"],
      ],
      { DELETE_TITLE: true },
    ),
    "added-over-180-days-ago": titles(
      [[facts("added_at"), "MORE_THAN_DAYS_AGO", "180"]],
      { UNMONITOR: true },
    ),
    "oversized-releases": titles(
      [[facts("total_file_size_bytes"), "GREATER_THAN", "40000000000"]],
      { SET_QUALITY_PROFILE: "Compact 1080p", SEARCH: true },
    ),
    "requested-media-expiry": titles(
      [
        [facts("requested"), "IS_TRUE"],
        [facts("tags"), "NOT_CONTAINS", "keep"],
      ],
      { DELETE_TITLE: true },
    ),
    "departed-requester": titles(
      [[facts("requested_by_usernames"), "CONTAINS", "departed-user"]],
      { DELETE_TITLE: true },
    ),
    "system-added-cleanup": titles(
      [
        [facts("added_by_user_id"), "HAS_NO_VALUE"],
        [facts("has_file"), "IS_TRUE"],
      ],
      { UNMONITOR: true },
    ),
    "watched-by-every-requester": titles(
      [
        [facts("requested"), "IS_TRUE"],
        [facts("watched_by_all_requesters"), "IS_TRUE"],
      ],
      { DELETE_TITLE: true },
    ),
    "tagged-for-removal": titles([[facts("tags"), "CONTAINS", "remove"]], {
      DELETE_TITLE: true,
    }),
    "flag-for-review": titles(
      [
        [facts("has_file"), "IS_TRUE"],
        [facts("first_imported_at"), "MORE_THAN_DAYS_AGO", "365"],
      ],
      { ADD_TAGS: ["needs-review"] },
    ),
    "no-quality-profile": titles(
      [[facts("quality_profile_id"), "HAS_NO_VALUE"]],
      { ADD_TAGS: ["needs-review"] },
    ),
    "keep-newest-three-downloaded-episodes": lifecycle(
      [],
      { DELETE_EPISODE_FILES: true },
      "EPISODE",
      [
        [
          "input.episode.facts.episode_position_by_air_date",
          "GREATER_THAN",
          "3",
        ],
      ],
    ),
    "keep-newest-two-downloaded-seasons": lifecycle(
      [],
      { DELETE_SEASON_FILES: true },
      "SEASON",
      [["input.season.facts.season_position_by_air_date", "GREATER_THAN", "2"]],
    ),
    "left-every-public-list": titles(
      [
        [facts("lists_left_all"), "IS_TRUE"],
        [facts("lists_left_handled"), "IS_TRUE"],
        [facts("lists_left_at"), "MORE_THAN_DAYS_AGO", "30"],
      ],
      { UNMONITOR: true },
    ),
    "list-added-never-watched": titles(
      [
        [facts("lists_added_by_list"), "IS_TRUE"],
        [facts("lists_days_on_list"), "GREATER_THAN", "90"],
        [facts("watched_by_user_ids"), "IS_EMPTY"],
      ],
      { ADD_TAGS: ["needs-review"] },
    ),
  },
  request: {
    "deny-adult-content": request([[facts("is_adult"), "IS_TRUE"]], {
      DENY: true,
      REASONS: ["adult_content"],
    }),
    "monthly-approval-quota": request(
      [[facts("approved_last_30d"), "GREATER_OR_EQUAL", "5"]],
      { MANUAL: true },
    ),
    "hold-approval-policy-list-requests": request(
      [["input.request.list.owner_list_policy", "EQUALS", "approval"]],
      { MANUAL: true },
    ),
    "deny-personal-list-above-1080p": request(
      [
        ["input.request.list.scope", "EQUALS", "personal"],
        [facts("quality_profile_max_resolution"), "GREATER_THAN", "1080"],
      ],
      { DENY: true },
    ),
  },
  scoring: {
    "block-old-releases": scoring(
      [["input.release.age_days", "GREATER_THAN", "365"]],
      { BLOCK: { code: "too_old" } },
    ),
    "block-hardcoded-subs": scoring(
      [["input.release.is_hardcoded_subs", "IS_TRUE"]],
      { BLOCK: { code: "hardcoded_subs" } },
    ),
  },
};

/// The builder model of a starter template, for the family the schema
/// builds; null for a template the builder cannot express, and before the
/// schema is read.
export function ruleTemplateBuilderModel(
  schema: RuleBuilderSchema | null,
  templateId: string,
): RuleBuilderModel | null {
  if (!schema) return null;
  return RULE_TEMPLATE_BUILDER_MODELS[schema.family]?.[templateId] ?? null;
}
