import type { ConfigFieldDef } from "./indexers";
import type { MediaServerProvider } from "./settings";
import type { TitleRecord } from "./titles";
import type { ProviderConfigValue } from "@/lib/utils/provider-config";

export type NotificationChannel = {
  id: string;
  name: string;
  channelType: string;
  mediaServerConnectionId: string | null;
  config: ProviderConfigValue[];
  storedSecretKeys: string[];
  isEnabled: boolean;
  blockedReason?: string | null;
  createdAt: string;
  updatedAt: string;
};

export type NotificationChannelDraft = {
  name: string;
  channelType: string;
  mediaServerConnectionId: string;
  isEnabled: boolean;
  configValues: Record<string, string>;
};

export type NotificationSubscription = {
  id: string;
  channelId: string | null;
  targetKind: NotificationTargetKind;
  targetId: string;
  eventType: string;
  scope: string;
  scopeId: string | null;
  isEnabled: boolean;
  createdAt: string;
  updatedAt: string;
};

export type NotificationTargetKind = "plugin_channel" | "media_server_connection";

export type NotificationTarget = {
  id: string;
  targetKind: NotificationTargetKind;
  name: string;
  providerType: string;
  mediaServerProvider: MediaServerProvider | null;
  mediaServerConnectionId: string | null;
  isEnabled: boolean;
};

export type NotificationSubscriptionDraft = {
  targetKind: NotificationTargetKind;
  targetId: string;
  eventTypes: string[];
  scope: string;
  facetScopeIds: string[];
  titleScopeId: string;
  titleScopeTitle: TitleRecord | null;
  isEnabled: boolean;
};

export type NotificationSubscriptionRow = {
  id: string;
  channelId: string | null;
  targetKind: NotificationTargetKind;
  targetId: string;
  eventTypes: string[];
  scope: string;
  scopeId: string | null;
  isEnabled: boolean;
  subscriptionIds: string[];
};

export type NotificationProviderType = {
  providerType: string;
  name: string;
  defaultBaseUrl: string | null;
  configFields: ConfigFieldDef[];
  supportedEvents: string[];
  supportsTest: boolean;
};
