import maintenanceContract from "../contracts/maintenance-input-contract.json" with { type: "json" };
import {
  ruleBuilderOperator as op,
  type RuleBuilderContract,
  type RuleBuilderSchema,
} from "./rule-builder.ts";

/// Maintenance rules: conditions read `input.facts`, a fact with nothing to
/// report is missing rather than null, and the outputs are the action heads
/// in the order the server applies them. Rules are scoped to titles, seasons
/// or episodes, and that scope is the builder variant.
export const MAINTENANCE_RULE_BUILDER_SCHEMA: RuleBuilderSchema = {
  family: "maintenance",
  idPrefix: "settings-maintenance-builder",
  preamble: "package rules\nimport rego.v1\n\n",
  helperRule: "rule_applies",
  contract: maintenanceContract as RuleBuilderContract,
  conditionSources: [{ section: "input.facts", label: "field" }],
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
    bool: {
      operators: [
        op("IS_TRUE", "none", "{path}"),
        op("IS_FALSE", "none", "not {path}"),
      ],
    },
    number: {
      operators: [
        op("EQUALS", "number", "{path} == {value}"),
        op("NOT_EQUALS", "number", "{path} != {value}"),
        op("GREATER_THAN", "number", "{path} > {value}"),
        op("GREATER_OR_EQUAL", "number", "{path} >= {value}"),
        op("LESS_THAN", "number", "{path} < {value}"),
        op("LESS_OR_EQUAL", "number", "{path} <= {value}"),
        op("HAS_VALUE", "none", "{path}"),
        op("HAS_NO_VALUE", "none", "not {path}"),
      ],
    },
    string: {
      operators: [
        op("EQUALS", "text", "{path} == {value}"),
        op("NOT_EQUALS", "text", "{path} != {value}"),
        op("HAS_VALUE", "none", "{path}"),
        op("HAS_NO_VALUE", "none", "not {path}"),
      ],
    },
    string_list: {
      operators: [
        op("CONTAINS", "text", "{value} in {path}"),
        op("NOT_CONTAINS", "text", "not {value} in {path}"),
        op("IS_EMPTY", "none", "count({path}) == 0"),
        op("IS_NOT_EMPTY", "none", "count({path}) > 0"),
      ],
    },
    timestamp: {
      operators: [
        op("MORE_THAN_DAYS_AGO", "days", "days_since({path}) > {value}"),
        op("LESS_THAN_DAYS_AGO", "days", "days_since({path}) < {value}"),
        op("HAS_VALUE", "none", "{path}"),
        op("HAS_NO_VALUE", "none", "not {path}"),
      ],
    },
  },
  outputs: [
    {
      id: "ADD_TAGS",
      head: "add_tags",
      labelKey: "settings.maintenanceSequenceStepAddTags",
      value: {
        kind: "text_list",
        options: "titleTags",
        placeholderKey: "settings.ruleBuilderTagsPlaceholder",
        emptyKey: "settings.ruleBuilderNoTags",
      },
      template: "{head} := [{items}] if {helper}",
    },
    {
      id: "REMOVE_TAGS",
      head: "remove_tags",
      labelKey: "settings.maintenanceSequenceStepRemoveTags",
      value: {
        kind: "text_list",
        options: "titleTags",
        placeholderKey: "settings.ruleBuilderTagsPlaceholder",
        emptyKey: "settings.ruleBuilderNoTags",
      },
      template: "{head} := [{items}] if {helper}",
    },
    {
      id: "SET_QUALITY_PROFILE",
      head: "set_quality_profile",
      labelKey: "settings.maintenanceSequenceStepChangeQualityProfile",
      value: {
        kind: "choice",
        options: "qualityProfiles",
        placeholderKey: "settings.ruleBuilderQualityProfilePlaceholder",
      },
      template: "{head} := {value} if {helper}",
    },
    {
      id: "MONITOR",
      head: "monitor",
      labelKey: "title.monitorAction",
      value: { kind: "flag" },
      template: "{head} if {helper}",
      // The server holds every subject that decides both of a pair, so a
      // rule whose only condition set declares both could never act.
      conflictsWith: ["UNMONITOR", "DELETE_FILES", "DELETE_TITLE"],
    },
    {
      id: "SEARCH",
      head: "search",
      labelKey: "settings.maintenanceSequenceStepSearch",
      value: { kind: "flag" },
      template: "{head} if {helper}",
    },
    {
      id: "FETCH_SUBTITLES",
      head: "fetch_subtitles",
      labelKey: "settings.maintenanceActionHeadFetchSubtitles",
      value: { kind: "flag" },
      template: "{head} if {helper}",
    },
    {
      id: "MOVE_TO_LIBRARY",
      head: "move_to_library",
      labelKey: "settings.maintenanceActionHeadMoveToLibrary",
      value: {
        kind: "choice",
        options: "libraries",
        placeholderKey: "settings.maintenanceBuilderLibraryPlaceholder",
      },
      template: "{head} := {value} if {helper}",
      variants: ["TITLE"],
      conflictsWith: ["MOVE_TO_ROOT", "DELETE_FILES", "DELETE_TITLE"],
    },
    {
      id: "MOVE_TO_ROOT",
      head: "move_to_root",
      labelKey: "settings.maintenanceActionHeadMoveToRoot",
      value: {
        kind: "choice",
        options: "storageRoots",
        placeholderKey: "settings.maintenanceBuilderRootPlaceholder",
        display: "path",
      },
      template: "{head} := {value} if {helper}",
      variants: ["TITLE"],
      conflictsWith: ["DELETE_FILES", "DELETE_TITLE"],
    },
    {
      id: "UNMONITOR",
      head: "unmonitor",
      labelKey: "settings.maintenanceSequenceStepUnmonitor",
      value: { kind: "flag" },
      template: "{head} if {helper}",
    },
    {
      id: "DELETE_FILES",
      head: "delete_files",
      labelKey: "settings.maintenanceSequenceStepDeleteFiles",
      value: { kind: "flag" },
      template: "{head} if {helper}",
      destructive: true,
    },
    {
      id: "DELETE_TITLE",
      head: "delete_title",
      labelKey: "settings.maintenanceSequenceStepDeleteTitleAndFiles",
      value: { kind: "flag" },
      template: "{head} if {helper}",
      destructive: true,
      variants: ["TITLE"],
    },
    {
      id: "NOTIFY",
      head: "notify",
      labelKey: "settings.maintenanceActionHeadNotify",
      value: {
        kind: "text",
        placeholderKey: "settings.maintenanceBuilderNotifyPlaceholder",
      },
      template: "{head} := {value} if {helper}",
    },
  ],
  outputRules: [{ rule: "at_least_one" }],
  labels: {
    outputs: "label.actions",
    outputsHelp: "settings.maintenanceBuilderActionsHelp",
    conditionsHelp: "settings.maintenanceBuilderConditionsHelp",
    outputNotInVariant: "settings.maintenanceBuilderActionTitleOnly",
    destructive: "settings.maintenanceTemplateDestructiveBadge",
  },
  legacy: {
    version: 1,
    fieldKey: "fact",
    fieldSection: "input.facts",
    outputsKey: "actions",
  },
};
