import type { JsonValue, ViewCategoryId } from "./quality-profiles";

export type DownloadClientRecord = {
  id: string;
  name: string;
  clientType: string;
  baseUrl: string | null;
  configJson: string;
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
  isEnabled: boolean;
};

export type DownloadClientTypeOption = {
  value: string;
  label: string;
};

export type DownloadClientConfigPayloadRecord = Record<string, JsonValue>;

export type DownloadClientRoutingSettings = {
  enabled: boolean;
  category: string;
  recentQueuePriority: string;
  olderQueuePriority: string;
  removeCompleted: boolean;
  removeFailed: boolean;
};

export type DownloadClientRoutingEntry = {
  clientId: string;
  enabled: boolean;
  category: string | null;
  recentQueuePriority: string | null;
  olderQueuePriority: string | null;
  removeCompleted: boolean;
  removeFailed: boolean;
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
