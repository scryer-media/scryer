import type { DownloadClientRoutingSettings } from "@/lib/types/download-clients";

export const DOWNLOAD_CLIENT_DEFAULT_CATEGORY_SETTING_KEY = "download_client.default_category";
export const LEGACY_NZBGET_CATEGORY_SETTING_KEY = "nzbget.category";
export const NZBGET_RECENT_PRIORITY_SETTING_KEY = "nzbget.recent_priority";
export const NZBGET_OLDER_PRIORITY_SETTING_KEY = "nzbget.older_priority";
export const NZBGET_REMOVE_COMPLETED_SETTING_KEY = "nzbget.remove_completed";
export const NZBGET_REMOVE_FAILED_SETTING_KEY = "nzbget.remove_failed";
export const DOWNLOAD_CLIENT_ROUTING_SETTINGS_KEY = "download_client.routing";
export const NZBGET_CLIENT_ROUTING_DEFAULT_ID = "__default";
export const DOWNLOAD_CLIENT_ROUTING_EMPTY: DownloadClientRoutingSettings = {
  enabled: true,
  category: "",
  recentQueuePriority: "",
  olderQueuePriority: "",
  removeCompleted: false,
  removeFailed: false,
};
export const NZBGET_CLIENT_ROUTING_EMPTY = DOWNLOAD_CLIENT_ROUTING_EMPTY;
export const MEDIA_SETTING_EMPTY_VALUE = "";
