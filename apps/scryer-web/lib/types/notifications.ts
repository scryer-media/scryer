import type { ConfigFieldDef } from "./indexers";

export type NotificationChannel = {
  id: string;
  name: string;
  channelType: string;
  configJson: string | null;
  isEnabled: boolean;
  createdAt: string;
  updatedAt: string;
};

export type NotificationChannelDraft = {
  name: string;
  channelType: string;
  isEnabled: boolean;
  configValues: Record<string, string>;
};

export type NotificationSubscription = {
  id: string;
  channelId: string;
  eventType: string;
  scope: string;
  scopeId: string | null;
  isEnabled: boolean;
  createdAt: string;
  updatedAt: string;
};

export type NotificationSubscriptionDraft = {
  channelId: string;
  eventType: string;
  scope: string;
  scopeId: string;
  isEnabled: boolean;
};

export type NotificationProviderType = {
  providerType: string;
  name: string;
  defaultBaseUrl: string | null;
  configFields: ConfigFieldDef[];
};
