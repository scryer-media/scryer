export type RuleSetRecord = {
  id: string;
  name: string;
  description: string;
  regoSource: string;
  enabled: boolean;
  priority: number;
  evaluationPhase?: "baseline" | "additional";
  disabledReason?: string | null;
  exclusiveGroup?: string | null;
  appliedFacets: string[];
  isManaged: boolean;
  managedKey: string | null;
  managedTagFilter: string[] | null;
  createdAt: string;
  updatedAt: string;
  /// The stored builder document, read only by the single-rule query. Opaque
  /// until `parseRuleBuilderModel` reads it.
  builderModel?: unknown;
};

export type RuleSetDraft = {
  name: string;
  description: string;
  regoSource: string;
  enabled: boolean;
  priority: number;
  appliedFacets: string[];
  /// Set while the draft is still exactly what the builder writes.
  builderModel: import("../utils/rule-builder").RuleBuilderModel | null;
};

export type RuleValidationResult = {
  valid: boolean;
  errors: string[];
  diagnostics?: import("../utils/rego-diagnostics").RuleValidationDiagnostic[];
};
