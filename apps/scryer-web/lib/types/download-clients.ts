import type { JsonValue, ViewCategoryId } from "./quality-profiles";
import type { ConfigFieldDef } from "./indexers";
import type { ProviderConfigValue } from "@/lib/utils/provider-config";

export type DownloadClientRecord = {
  id: string;
  name: string;
  clientType: string;
  baseUrl: string | null;
  config: ProviderConfigValue[];
  storedSecretKeys: string[];
  /**
   * Proxy carrying this client's traffic, or null when none is assigned. Any
   * proxy kind may be assigned.
   */
  proxyConfigId: string | null;
  isEnabled: boolean;
  status: string;
  lastError: string | null;
  lastSeenAt: string | null;
  createdAt: string;
  updatedAt: string;
};

export type DownloadClientDraft = {
  name: string;
  clientType: string;
  host: string;
  port: string;
  urlBase: string;
  useSsl: boolean;
  apiKey: string;
  username: string;
  password: string;
  remotePathMappings: string;
  configValues: Record<string, string>;
  /** Proxy assignment; null is "direct". */
  proxyConfigId: string | null;
  isEnabled: boolean;
};

export type DownloadClientTypeOption = {
  value: string;
  label: string;
  configFields?: ConfigFieldDef[];
  defaultBaseUrl?: string | null;
};

export type DownloadClientConfigPayloadRecord = Record<string, JsonValue>;

export type DownloadClientRoutingSettings = {
  enabled: boolean;
  category: string;
  recentQueuePriority: string;
  olderQueuePriority: string;
  removeCompleted: boolean;
  removeFailed: boolean;
  /** Default seeding profile for grabs routed here. null inherits the global default. */
  seedingProfileId: string | null;
};

export type DownloadClientRoutingEntry = {
  clientId: string;
  enabled: boolean;
  category: string | null;
  recentQueuePriority: string | null;
  olderQueuePriority: string | null;
  removeCompleted: boolean;
  removeFailed: boolean;
  seedingProfileId: string | null;
};

export type DownloadClientRoutingSettingsByClient = Record<
  string,
  DownloadClientRoutingSettings
>;

export type DownloadClientRoutingSettingsByScope = Record<
  ViewCategoryId,
  DownloadClientRoutingSettingsByClient
>;

export type NzbgetCategoryRoutingSettings = DownloadClientRoutingSettings;
export type NzbgetClientRoutingSettingsByClient = DownloadClientRoutingSettingsByClient;
export type NzbgetClientRoutingSettingsByScope = DownloadClientRoutingSettingsByScope;
