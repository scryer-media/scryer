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
  createdAt: string;
  updatedAt: string;
};

/**
 * Every indexer proxy provider the API accepts, in the order the editor lists
 * them: challenge solvers first, then transport proxies.
 */
export const INDEXER_PROXY_PROVIDER_TYPES = [
  "byparr",
  "trawl",
  "http",
  "socks4",
  "socks5",
] as const;

export type IndexerProxyProviderTypeValue =
  (typeof INDEXER_PROXY_PROVIDER_TYPES)[number];

const TRANSPORT_INDEXER_PROXY_PROVIDER_TYPES: ReadonlySet<string> = new Set([
  "http",
  "socks4",
  "socks5",
]);

export function isIndexerProxyProviderType(
  value: string,
): value is IndexerProxyProviderTypeValue {
  return (INDEXER_PROXY_PROVIDER_TYPES as readonly string[]).includes(value);
}

/**
 * Transport proxies carry indexer traffic; challenge solvers answer browser
 * challenges. The two kinds take entirely different settings.
 */
export function isTransportIndexerProxyProvider(providerType: string): boolean {
  return TRANSPORT_INDEXER_PROXY_PROVIDER_TYPES.has(providerType);
}

/**
 * Which providers accept a username and password. Challenge solvers take none,
 * and SOCKS4 is rejected too: the HTTP client builds its SOCKS4 connector
 * without auth, so a credential would be silently dropped on the wire.
 */
export function supportsIndexerProxyCredentials(providerType: string): boolean {
  return providerType === "http" || providerType === "socks5";
}

/**
 * Remote DNS is the `socks4a` / `socks5h` behaviour, so it is a SOCKS-only
 * choice. An HTTP CONNECT proxy always resolves the destination itself, and a
 * solver fetches the page entirely on its own side.
 */
export function supportsIndexerProxyRemoteDns(providerType: string): boolean {
  return providerType === "socks4" || providerType === "socks5";
}

export type IndexerProxyRecord = {
  id: string;
  name: string;
  providerType: string;
  /** Null for transport proxies, which speak no challenge-solver protocol. */
  protocol: string | null;
  baseUrl: string;
  requestTimeoutSeconds: number;
  /** Whether a username or password is stored, never the values themselves. */
  hasCredentials: boolean;
  /** SOCKS5 only: destination hostnames are resolved at the proxy. */
  remoteDns: boolean;
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
  providerType: IndexerProxyProviderTypeValue;
  name: string;
  baseUrl: string;
  requestTimeoutSeconds: number;
  /**
   * Write-only transport-proxy credentials. Blank on an edit means "leave the
   * stored secret alone" — they are never read back from the API.
   */
  username: string;
  password: string;
  /** Whether the proxy being edited already has a stored credential. */
  hasStoredCredentials: boolean;
  /** Explicitly drop the stored credentials instead of leaving them alone. */
  clearCredentials: boolean;
  /** SOCKS5 only: resolve destination hostnames at the proxy (`socks5h`). */
  remoteDns: boolean;
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
