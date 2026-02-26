import type { NzbgetCategoryRoutingSettings } from "@/lib/types/download-clients";

export const NZBGET_CATEGORY_SETTING_KEY = "nzbget.category";
export const NZBGET_RECENT_PRIORITY_SETTING_KEY = "nzbget.recent_priority";
export const NZBGET_OLDER_PRIORITY_SETTING_KEY = "nzbget.older_priority";
export const NZBGET_REMOVE_COMPLETED_SETTING_KEY = "nzbget.remove_completed";
export const NZBGET_REMOVE_FAILED_SETTING_KEY = "nzbget.remove_failed";
export const NZBGET_TAGS_SETTING_KEY = "nzbget.tags";
export const NZBGET_CLIENT_ROUTING_SETTINGS_KEY = "nzbget.client_routing";
export const NZBGET_CLIENT_ROUTING_DEFAULT_ID = "__default";
export const NZBGET_CLIENT_ROUTING_EMPTY: NzbgetCategoryRoutingSettings = {
  category: "",
  recentPriority: "",
  olderPriority: "",
  removeCompleted: false,
  removeFailed: false,
  tags: [],
};
export const MEDIA_SETTING_EMPTY_VALUE = "";
