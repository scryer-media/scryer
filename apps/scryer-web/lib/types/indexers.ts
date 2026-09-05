import type { ViewCategoryId } from "./quality-profiles";
import type { ProviderConfigValue } from "@/lib/utils/provider-config";

export type IndexerRecord = {
  id: string;
  name: string;
  providerType: string;
  baseUrl: string;
  proxyConfigId: string | null;
  downloadClientId: string | null;
  /** Seeding profile assigned to this indexer. null inherits the routing/global default. */
  seedingProfileId: string | null;
  /**
   * Whether Prowlarr supplied seed criteria for this managed child. When it did
   * and no profile is assigned, those criteria apply; picking a profile
   * overrides them.
   */
  hasProwlarrSeedCriteria: boolean;
  /**
   * Minimum seeders Prowlarr imported for this managed child, or null when it
   * supplied none. Read-only: 0 means Prowlarr turned the seeder check off, and
   * assigning a profile overrides the imported value.
   */
  prowlarrMinimumSeeders: number | null;
  hasApiKey: boolean;
  storedSecretKeys: string[];
  rateLimitSeconds: number | null;
  rateLimitBurst: number | null;
  disabledUntil: string | null;
  isEnabled: boolean;
  isManaged: boolean;
  managedParentConfigId: string | null;
  supportsManagedChildrenSync: boolean;
  enableInteractiveSearch: boolean;
  enableAutoSearch: boolean;
  lastHealthStatus: string | null;
  lastErrorMessage: string | null;
  lastErrorAt: string | null;
  lastQueryAt: string | null;
  config: ProviderConfigValue[];
  createdAt: string;
  updatedAt: string;
};

export type IndexerDraft = {
  name: string;
  providerType: string;
  proxyConfigId: string | null;
  downloadClientId: string | null;
  seedingProfileId: string | null;
  storedSecretKeys: string[];
  isEnabled: boolean;
  enableInteractiveSearch: boolean;
  enableAutoSearch: boolean;
  configValues: Record<string, string>;
};

export type ConfigFieldOption = {
  value: string;
  label: string;
  configOverrides?: Array<{ key: string; value: string }>;
};

export type ConfigFieldTypeValue =
  | "STRING"
  | "PASSWORD"
  | "MULTILINE"
  | "BOOL"
  | "SELECT"
  | "FILTERED_SELECT"
  | "NUMBER"
  | "PATH"
  | "TAG";

export type ConditionOpValue = "EQ" | "NE" | "IN" | "NOT_IN" | "NON_EMPTY";

/// A predicate over another field's current value.
///
/// `EQ`/`NE` compare against the first entry of `values`, `IN`/`NOT_IN` against
/// the whole set, and `NON_EMPTY` ignores it.
export type FieldCondition = {
  key: string;
  op: ConditionOpValue;
  values: string[];
};

export type ConfigFieldValueSourceValue = "USER" | "HOST_BINDING";
export type ConfigFieldRoleValue = "CONNECTION_URL";

export type ConfigFieldDef = {
  key: string;
  label: string;
  fieldType: ConfigFieldTypeValue;
  required: boolean;
  defaultValue: string | null;
  valueSource: ConfigFieldValueSourceValue;
  role: ConfigFieldRoleValue | null;
  hostBinding: string | null;
  options: ConfigFieldOption[];
  helpText: string | null;
  /// Shown only while this holds; null means always shown.
  visibleWhen: FieldCondition | null;
  /// Required while this holds, on top of `required`. A field hidden by
  /// `visibleWhen` is never required, whatever this says.
  requiredWhen: FieldCondition | null;
  /// Belongs behind the form's advanced disclosure rather than shown up front.
  advanced: boolean;
};

export type ProviderTypeInfo = {
  providerType: string;
  name: string;
  defaultBaseUrl: string | null;
  configFields: ConfigFieldDef[];
  availableHostBindings: string[];
  recommendedFacets: Array<"MOVIE" | "SERIES" | "ANIME">;
};

/// Which of a provider's declared fields the form offers.
///
/// Host-bound values are supplied by the host, so they are configuration the
/// operator never sees. Everything else is the plugin's to decide — this used
/// to take a provider type and filter on it, and nothing should reintroduce
/// that: a form that knows provider names is a form that drifts from them.
export function visibleIndexerConfigFields(
  configFields: ConfigFieldDef[],
): ConfigFieldDef[] {
  return configFields.filter((field) => field.valueSource !== "HOST_BINDING");
}

export type IndexerCategoryRoutingSettings = {
  categories: string[];
  enabled: boolean;
  priority: number;
};

export type IndexerRoutingEntry = {
  indexerId: string;
  enabled: boolean;
  categories: string[];
  priority: number;
};

export type IndexerRoutingSettingsByIndexer = Record<string, IndexerCategoryRoutingSettings>;

export type IndexerRoutingSettingsByScope = Record<ViewCategoryId, IndexerRoutingSettingsByIndexer>;
