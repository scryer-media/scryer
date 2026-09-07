import type { ViewCategoryId } from "./quality-profiles";
import type { ProviderConfigValue } from "@/lib/utils/provider-config";

export type IndexerRecord = {
  id: string;
  name: string;
  providerType: string;
  baseUrl: string;
  indexerProxyConfigId: string | null;
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
  /**
   * Categories from the indexer's most recent caps snapshot, sorted by code.
   * Empty until the first successful caps refresh.
   */
  capsCategories: IndexerCapsCategory[];
  createdAt: string;
  updatedAt: string;
};

export type IndexerCapsCategory = {
  /** Category id exactly as the indexer expects it in the `cat` parameter. */
  code: string;
  /** Human-readable name from the caps document, or null when it has none. */
  label: string | null;
};

export type IndexerProxyRecord = {
  id: string;
  name: string;
  providerType: string;
  protocol: string;
  baseUrl: string;
  requestTimeoutSeconds: number;
  isEnabled: boolean;
  lastHealthStatus: string | null;
  lastErrorMessage: string | null;
  lastErrorAt: string | null;
  createdAt: string;
  updatedAt: string;
};

export type IndexerDraft = {
  name: string;
  providerType: string;
  indexerProxyConfigId: string | null;
  downloadClientId: string | null;
  seedingProfileId: string | null;
  storedSecretKeys: string[];
  isEnabled: boolean;
  enableInteractiveSearch: boolean;
  enableAutoSearch: boolean;
  configValues: Record<string, string>;
};

export type IndexerProxyDraft = {
  providerType: "byparr" | "trawl";
  name: string;
  baseUrl: string;
  requestTimeoutSeconds: number;
  isEnabled: boolean;
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
  | "NUMBER"
  | "PATH"
  | "TAG";

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
};

export type ProviderTypeInfo = {
  providerType: string;
  name: string;
  defaultBaseUrl: string | null;
  configFields: ConfigFieldDef[];
  availableHostBindings: string[];
  recommendedFacets: Array<"MOVIE" | "SERIES" | "ANIME">;
};

export function visibleIndexerConfigFields(
  _providerType: string,
  configFields: ConfigFieldDef[],
): ConfigFieldDef[] {
  return configFields;
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
