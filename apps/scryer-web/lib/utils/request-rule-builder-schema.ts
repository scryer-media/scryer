import requestContract from "../contracts/request-input-contract.json" with { type: "json" };
import { NULL_AWARE_RULE_BUILDER_KINDS } from "./rule-builder-kinds.ts";
import type {
  RuleBuilderBaseSchema,
  RuleBuilderConditionSource,
  RuleBuilderContract,
} from "./rule-builder.ts";

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
/// requester, the target library and the clock. A fact the host does not
/// have is left out of `input.facts`, never sent as `null`; a rule reading a
/// fact the host could not look up is held before it runs, so a missing fact
/// is one the source confirmed has no value. The request's own optional
/// fields are `null` when not set. What a rule can do comes from the server.
/// The requester, picked by account. A rule that reads it names people, which
/// only an author who can manage permissions may save.
const REQUESTER_SOURCE: RuleBuilderConditionSource = {
  section: "input.requester",
  label: "path",
  // The account is picked by name; its username and timestamps add nothing a
  // builder comparison can use.
  exclude: ["username", "created_at", "last_login_at"],
  fields: { user_id: { options: "users" } },
};

export const REQUEST_RULE_BUILDER_BASE: RuleBuilderBaseSchema = {
  family: "request",
  idPrefix: "settings-request-builder",
  preamble: "package rules\nimport rego.v1\n\n",
  helperRule: "rule_applies",
  contract: requestContract as RuleBuilderContract,
  conditionSources: [
    {
      section: "input.facts",
      label: "path",
      // The host leaves these out when the title has none.
      fields: {
        exists_in_library_ids: { options: "libraryIds" },
        genres: { kind: "omitted_when_empty_string_list" },
        canonical_tag_keys: { kind: "omitted_when_empty_string_list" },
        themes: { kind: "omitted_when_empty_string_list" },
        quality_profile_tiers: { kind: "omitted_when_empty_string_list" },
      },
    },
    {
      section: "input.request",
      label: "path",
      // Facts that hold an id are picked by name.
      fields: {
        quality_profile_id: { options: "qualityProfileIds" },
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
    REQUESTER_SOURCE,
    {
      section: "input.request.list",
      label: "path",
      fields: { subscription_id: { options: "lists" } },
    },
    {
      section: "input.library",
      label: "path",
      fields: {
        id: { options: "libraryIds" },
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
  labels: {
    outputs: "settings.requestBuilderOutputs",
    outputsHelp: "settings.requestBuilderOutputsHelp",
  },
};

/// The request builder for an author who cannot save a rule that names people:
/// the requester is not offered.
export const REQUEST_RULE_BUILDER_BASE_WITHOUT_REQUESTER: RuleBuilderBaseSchema =
  {
    ...REQUEST_RULE_BUILDER_BASE,
    conditionSources: REQUEST_RULE_BUILDER_BASE.conditionSources.filter(
      (source) => source !== REQUESTER_SOURCE,
    ),
  };
