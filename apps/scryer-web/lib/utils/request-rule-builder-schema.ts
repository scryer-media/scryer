import requestContract from "../contracts/request-input-contract.json" with { type: "json" };
import { NULL_AWARE_RULE_BUILDER_KINDS } from "./rule-builder-kinds.ts";
import type { RuleBuilderContract, RuleBuilderSchema } from "./rule-builder.ts";

const WEEKDAYS = [
  "monday",
  "tuesday",
  "wednesday",
  "thursday",
  "friday",
  "saturday",
  "sunday",
].map((day) => ({ value: day, labelKey: `script.schedule.day.${day}` }));

/// Request rules: conditions read the title's facts, the request itself, the
/// target library and the clock. Requester fields are left to Advanced,
/// because reading them makes a rule person-targeted. An unknown value is
/// `null`. A rule votes approve, deny or manual review, and may add tags and
/// reason codes.
export const REQUEST_RULE_BUILDER_SCHEMA: RuleBuilderSchema = {
  family: "request",
  idPrefix: "settings-request-builder",
  preamble: "package rules\nimport rego.v1\n\n",
  helperRule: "rule_applies",
  contract: requestContract as RuleBuilderContract,
  conditionSources: [
    { section: "input.facts", label: "path" },
    {
      section: "input.request",
      label: "path",
      fields: {
        origin: {
          kind: "choice",
          choices: [
            {
              value: "manual",
              labelKey: "settings.requestBuilderOriginManual",
            },
            {
              value: "public_list",
              labelKey: "settings.requestBuilderOriginPublicList",
            },
            {
              value: "personal_list",
              labelKey: "settings.requestBuilderOriginPersonalList",
            },
          ],
        },
      },
    },
    { section: "input.request.external_ids", label: "path" },
    { section: "input.request.list", label: "path" },
    {
      section: "input.library",
      label: "path",
      fields: {
        facet: {
          kind: "choice",
          choices: [
            { value: "movie", labelKey: "label.movie" },
            { value: "series", labelKey: "label.series" },
            { value: "anime", labelKey: "label.anime" },
          ],
        },
      },
    },
    {
      section: "input.now",
      label: "path",
      fields: { weekday: { kind: "choice", choices: WEEKDAYS } },
    },
  ],
  typeRules: [
    { type: "bool", kind: "bool" },
    { type: "number", kind: "number" },
    { type: "number?", kind: "nullable_number" },
    { type: "string", kind: "string" },
    { type: "string?", kind: "nullable_string" },
    { type: "string[]", kind: "string_list" },
    { type: "string[]?", kind: "nullable_string_list" },
  ],
  kinds: NULL_AWARE_RULE_BUILDER_KINDS,
  outputs: [
    {
      id: "APPROVE",
      head: "approve",
      labelKey: "settings.requestBuilderApprove",
      value: { kind: "flag" },
      template: "{head} if {helper}",
    },
    {
      id: "DENY",
      head: "deny",
      labelKey: "settings.requestBuilderDeny",
      value: { kind: "flag" },
      template: "{head} if {helper}",
    },
    {
      id: "MANUAL",
      head: "manual",
      labelKey: "settings.requestBuilderManual",
      value: { kind: "flag" },
      template: "{head} if {helper}",
    },
    {
      // Tags land on real titles, so they keep the shape the server accepts
      // from a rule: at most 16, each 1 to 64 letters, digits, '.', '_', '-'
      // or spaces.
      id: "TAGS",
      head: "tags",
      labelKey: "settings.requestBuilderTags",
      value: {
        kind: "text_list",
        options: "titleTags",
        placeholderKey: "settings.ruleBuilderTagsPlaceholder",
        emptyKey: "settings.ruleBuilderNoTags",
        maxItems: 16,
        maxBytes: 64,
        pattern: "[A-Za-z0-9._ -]+",
      },
      template: "{head} contains {item} if {helper}",
      perItem: true,
    },
    {
      // The server keeps at most 32 reason codes of up to 120 bytes each.
      id: "REASONS",
      head: "reasons",
      labelKey: "settings.requestBuilderReasons",
      value: {
        kind: "text_list",
        placeholderKey: "settings.requestBuilderReasonsPlaceholder",
        maxItems: 32,
        maxBytes: 120,
      },
      template: "{head} contains {item} if {helper}",
      perItem: true,
    },
  ],
  outputRules: [
    // A rule that writes only reasons never contributes to a decision, so
    // the server refuses it.
    { rule: "at_least_one", of: ["APPROVE", "DENY", "MANUAL", "TAGS"] },
    // Manual beats deny beats approve within one rule, so a second vote
    // would never count.
    { rule: "at_most_one", of: ["APPROVE", "DENY", "MANUAL"] },
  ],
  labels: {
    outputs: "settings.requestBuilderOutputs",
    outputsHelp: "settings.requestBuilderOutputsHelp",
    conditionsHelp: "settings.requestBuilderConditionsHelp",
  },
};
