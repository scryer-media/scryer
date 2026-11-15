// Whole builder schemas from the checked-in served-output files, for tests.
// The files stand in for what the `ruleBuilderSchema` query serves; they are
// written by hand until the server produces them.

import maintenanceServed from "../contracts/maintenance-builder-schema.json" with { type: "json" };
import requestServed from "../contracts/request-builder-schema.json" with { type: "json" };
import scoringServed from "../contracts/scoring-builder-schema.json" with { type: "json" };
import { MAINTENANCE_RULE_BUILDER_BASE } from "./maintenance-rule-builder-schema.ts";
import { REQUEST_RULE_BUILDER_BASE } from "./request-rule-builder-schema.ts";
import {
  ruleBuilderSchemaFromPayload,
  type RuleBuilderSchema,
  type RuleBuilderSchemaPayload,
} from "./rule-builder.ts";
import { SCORING_RULE_BUILDER_SCHEMA } from "./scoring-rule-builder-schema.ts";

export const MAINTENANCE_SERVED_PAYLOAD =
  maintenanceServed as RuleBuilderSchemaPayload;
export const REQUEST_SERVED_PAYLOAD = requestServed as RuleBuilderSchemaPayload;
export const SCORING_SERVED_PAYLOAD = scoringServed as RuleBuilderSchemaPayload;

export const MAINTENANCE_CONTRACT_SCHEMA: RuleBuilderSchema =
  ruleBuilderSchemaFromPayload(
    MAINTENANCE_RULE_BUILDER_BASE,
    MAINTENANCE_SERVED_PAYLOAD,
  );

export const REQUEST_CONTRACT_SCHEMA: RuleBuilderSchema =
  ruleBuilderSchemaFromPayload(REQUEST_RULE_BUILDER_BASE, REQUEST_SERVED_PAYLOAD);

/// Scoring serves no outputs, so its builder is the static schema.
export const SCORING_CONTRACT_SCHEMA: RuleBuilderSchema =
  SCORING_RULE_BUILDER_SCHEMA;
