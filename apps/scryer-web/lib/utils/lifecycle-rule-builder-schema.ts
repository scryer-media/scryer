import lifecycleContract from "../contracts/lifecycle-input-contract.json" with { type: "json" };
import { NULL_AWARE_RULE_BUILDER_KINDS } from "./rule-builder-kinds.ts";
import {
  ruleBuilderOperator as op,
  type RuleBuilderBaseSchema,
  type RuleBuilderContract,
} from "./rule-builder.ts";

/// Lifecycle rules: conditions read `input.facts`, where a fact with nothing
/// to report is missing rather than null. Each action acts on the title, a
/// season or an episode, the subject the server lists it under; a season or
/// episode action may also read that subject's own facts. The actions come
/// from the server, in the order it applies them.
export const LIFECYCLE_RULE_BUILDER_BASE: RuleBuilderBaseSchema = {
  family: "lifecycle",
  idPrefix: "settings-lifecycle-builder",
  preamble: "package rules\nimport rego.v1\n\n",
  helperRule: "rule_applies",
  contract: lifecycleContract as RuleBuilderContract,
  conditionSources: [
    {
      section: "input.facts",
      label: "field",
      // Facts that hold an id are picked by name.
      fields: {
        added_by_user_id: { options: "users" },
        requested_by_user_ids: { options: "users" },
        watched_by_user_ids: { options: "users" },
        watchlisted_by_user_ids: { options: "users" },
        favorited_by_user_ids: { options: "users" },
        quality_profile_id: { options: "qualityProfileIds" },
        lists_enabled_ids: { options: "lists" },
      },
    },
  ],
  typeRules: [
    { type: "bool", kind: "bool" },
    { type: "number", kind: "number" },
    { type: "string[]", kind: "string_list" },
    // Timestamps travel as RFC 3339 strings, and every timestamp fact in the
    // contract is named for the moment it records.
    { type: "string", suffix: "_at", kind: "timestamp" },
    { type: "string", kind: "string" },
  ],
  kinds: {
    bool: NULL_AWARE_RULE_BUILDER_KINDS.bool,
    number: NULL_AWARE_RULE_BUILDER_KINDS.nullable_number,
    string: NULL_AWARE_RULE_BUILDER_KINDS.nullable_string,
    string_list: NULL_AWARE_RULE_BUILDER_KINDS.string_list,
    timestamp: {
      operators: [
        op("MORE_THAN_DAYS_AGO", "days", "days_since({path}) > {value}"),
        // A moment still to come is not "less than N days ago".
        op(
          "LESS_THAN_DAYS_AGO",
          "days",
          "days_since({path}) >= 0\ndays_since({path}) < {value}",
        ),
        op("HAS_VALUE", "none", "{path} != null"),
        op("HAS_NO_VALUE", "none", "not {path} != null"),
      ],
    },
  },
  labels: {
    outputs: "label.actions",
    outputsHelp: "settings.lifecycleBuilderActionsHelp",
    conditionsHelp: "settings.lifecycleBuilderConditionsHelp",
  },
};
