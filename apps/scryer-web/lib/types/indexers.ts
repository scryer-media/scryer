import type { ViewCategoryId } from "./quality-profiles";

export type IndexerRecord = {
  id: string;
  name: string;
  providerType: string;
  baseUrl: string;
  hasApiKey: boolean;
  rateLimitSeconds: number | null;
  rateLimitBurst: number | null;
  disabledUntil: string | null;
  isEnabled: boolean;
  enableInteractiveSearch: boolean;
  enableAutoSearch: boolean;
  lastHealthStatus: string | null;
  lastErrorAt: string | null;
  createdAt: string;
  updatedAt: string;
};

export type IndexerDraft = {
  name: string;
  providerType: string;
  baseUrl: string;
  apiKey: string;
  rateLimitSeconds: string;
  rateLimitBurst: string;
  isEnabled: boolean;
  enableInteractiveSearch: boolean;
  enableAutoSearch: boolean;
};

export type IndexerCategoryRoutingSettings = {
  categories: string[];
  enabled: boolean;
  priority: number;
};

export type IndexerRoutingSettingsByIndexer = Record<string, IndexerCategoryRoutingSettings>;

export type IndexerRoutingSettingsByScope = Record<ViewCategoryId, IndexerRoutingSettingsByIndexer>;
