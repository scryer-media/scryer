export function subscribableProviderNotificationEvents(
  hostEventTypes: readonly string[],
  providerSupportedEvents: readonly string[],
): string[] {
  if (providerSupportedEvents.length === 0) {
    return [...hostEventTypes];
  }

  const providerEvents = new Set(providerSupportedEvents);
  return hostEventTypes.filter((eventType) => providerEvents.has(eventType));
}

/** Translation key of each notification event's name. */
const NOTIFICATION_EVENT_LABEL_KEYS: Record<string, string> = {
  grab: "settings.notificationEvent.grab",
  download: "settings.notificationEvent.download",
  upgrade: "settings.notificationEvent.upgrade",
  import_complete: "settings.notificationEvent.importComplete",
  import_rejected: "settings.notificationEvent.importRejected",
  rename: "settings.notificationEvent.rename",
  title_added: "settings.notificationEvent.titleAdded",
  title_deleted: "settings.notificationEvent.titleDeleted",
  title_moved: "settings.notificationEvent.titleMoved",
  file_deleted: "settings.notificationEvent.fileDeleted",
  file_deleted_for_upgrade: "settings.notificationEvent.fileDeletedForUpgrade",
  file_restored: "settings.notificationEvent.fileRestored",
  post_processing_completed:
    "settings.notificationEvent.postProcessingCompleted",
  subtitle_downloaded: "settings.notificationEvent.subtitleDownloaded",
  subtitle_search_failed: "settings.notificationEvent.subtitleSearchFailed",
  media_request_submitted: "settings.notificationEvent.mediaRequestSubmitted",
  media_request_approved: "settings.notificationEvent.mediaRequestApproved",
  media_request_rejected: "settings.notificationEvent.mediaRequestRejected",
  media_request_canceled: "settings.notificationEvent.mediaRequestCanceled",
  health_issue: "settings.notificationEvent.healthIssue",
  health_restored: "settings.notificationEvent.healthRestored",
  application_update: "settings.notificationEvent.applicationUpdate",
  manual_interaction_required:
    "settings.notificationEvent.manualInteractionRequired",
  list_title_added: "settings.notificationEvent.listTitleAdded",
  list_request_submitted: "settings.notificationEvent.listRequestSubmitted",
  list_item_held: "settings.notificationEvent.listItemHeld",
  list_title_left: "settings.notificationEvent.listTitleLeft",
  list_sync_failed: "settings.notificationEvent.listSyncFailed",
  list_unfollowed: "settings.notificationEvent.listUnfollowed",
  lifecycle_rule_notified: "settings.notificationEvent.lifecycleRuleNotified",
  test: "settings.notificationEvent.test",
};

export function humanizeSnakeCase(value: string) {
  return value
    .split("_")
    .filter(Boolean)
    .map((segment) => segment.charAt(0).toUpperCase() + segment.slice(1))
    .join(" ");
}

/** A notification event's translated name, or its id in words when it has none. */
export function notificationEventLabel(
  eventType: string,
  t: (key: string) => string,
) {
  const key = NOTIFICATION_EVENT_LABEL_KEYS[eventType];
  return key ? t(key) : humanizeSnakeCase(eventType);
}
