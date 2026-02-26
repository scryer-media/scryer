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

export type DownloadClientConfigPayloadRecord = Record<string, JsonValue>;

export type NzbgetCategoryRoutingSettings = {
  category: string;
  recentPriority: string;
  olderPriority: string;
  removeCompleted: boolean;
  removeFailed: boolean;
  tags: string[];
};

export type NzbgetClientRoutingSettingsByClient = Record<
  string,
  NzbgetCategoryRoutingSettings
>;

export type NzbgetClientRoutingSettingsByScope = Record<
  ViewCategoryId,
  NzbgetClientRoutingSettingsByClient
>;
