import scoringContract from "../contracts/rule-input-contract.json" with { type: "json" };
import { NULL_AWARE_RULE_BUILDER_KINDS } from "./rule-builder-kinds.ts";
import type {
  RuleBuilderBaseSchema,
  RuleBuilderContract,
  RuleBuilderSchema,
} from "./rule-builder.ts";

/// The server keeps a score entry's points in a 32-bit integer.
const SCORE_POINTS_MIN = -2_147_483_648;
const SCORE_POINTS_MAX = 2_147_483_647;

/// A score code the builder writes: lowercase words joined by underscores,
/// the shape every shipped rule uses for the codes it reports.
const SCORE_CODE = {
  id: "code",
  kind: "text",
  labelKey: "settings.scoringBuilderCode",
  placeholderKey: "settings.scoringBuilderCodePlaceholder",
  pattern: "[a-z][a-z0-9_]*",
  maxBytes: 64,
} as const;

/// Scoring rules: conditions read the release, the quality profile, the
/// title's context, the built-in score and, after download, the file. An
/// unknown value is `null`. A rule adds one score entry under a code, or
/// blocks the release.
export const SCORING_RULE_BUILDER_BASE: RuleBuilderBaseSchema = {
  family: "scoring",
  idPrefix: "settings-scoring-builder",
  preamble: "package rules\nimport rego.v1\n\n",
  helperRule: "rule_applies",
  contract: scoringContract as RuleBuilderContract,
  conditionSources: [
    { section: "input.release", label: "path" },
    { section: "input.release.stereoscopy", label: "path" },
    { section: "input.profile", label: "path" },
    { section: "input.profile.scoring_overrides", label: "path" },
    { section: "input.context", label: "path" },
    { section: "input.builtin_score", label: "path" },
    { section: "input.file", label: "path" },
  ],
  typeRules: [
    { type: "bool", kind: "bool" },
    { type: "bool?", kind: "bool" },
    { type: "number", kind: "number" },
    { type: "int", kind: "number" },
    { type: "float", kind: "number" },
    { type: "number?", kind: "nullable_number" },
    { type: "string", kind: "string" },
    { type: "string?", kind: "nullable_string" },
    { type: "string[]", kind: "string_list" },
  ],
  kinds: NULL_AWARE_RULE_BUILDER_KINDS,
  labels: {
    outputs: "settings.scoringBuilderOutputs",
    outputsHelp: "settings.scoringBuilderOutputsHelp",
    conditionsHelp: "settings.scoringBuilderConditionsHelp",
  },
};

/// Scoring's own outputs, used while the server serves none for scoring: a
/// score entry is written under a code with two typed parts, which a served
/// output cannot describe yet.
export const SCORING_RULE_BUILDER_SCHEMA: RuleBuilderSchema = {
  ...SCORING_RULE_BUILDER_BASE,
  outputs: [
    {
      id: "ADJUST_SCORE",
      head: "score_entry",
      labelKey: "settings.scoringBuilderAdjustScore",
      value: {
        kind: "structured",
        parts: [
          SCORE_CODE,
          {
            id: "points",
            kind: "number",
            labelKey: "settings.scoringBuilderPoints",
            min: SCORE_POINTS_MIN,
            max: SCORE_POINTS_MAX,
          },
        ],
      },
      template: "{head}[{part.code}] := {part.points} if {helper}",
    },
    {
      id: "BLOCK",
      head: "score_entry",
      labelKey: "settings.scoringBuilderBlock",
      value: { kind: "structured", parts: [SCORE_CODE] },
      template: "{head}[{part.code}] := scryer.block_score() if {helper}",
    },
  ],
  outputRules: [{ rule: "exactly_one", of: ["ADJUST_SCORE", "BLOCK"] }],
};
