import { subscribableProviderNotificationEvents } from "../notification-capabilities.ts";

/**
 * Events a new media-server connection is offered a library refresh for.
 * Upgrade and File Deleted for Upgrade are left out: Import Complete already
 * carries an upgrade's replaced paths, so subscribing to them would refresh
 * the same files again.
 */
export const MEDIA_SERVER_REFRESH_OFFER_EVENTS = [
  "import_complete",
  "rename",
  "file_deleted",
  "title_moved",
] as const;

export type MediaServerRefreshProvider = {
  providerType: string;
  supportedEvents: readonly string[];
};

export type MediaServerRefreshOffer =
  /** The server's notification plugin is installed and handles these events. */
  | { kind: "available"; eventTypes: string[] }
  /** The server's notification plugin is not installed. */
  | { kind: "plugin_missing" }
  /** The plugin is installed but handles none of the offered events. */
  | { kind: "none" };

/**
 * What the create-connection form can offer for a media-server provider,
 * given the installed notification providers. The provider value is the
 * connection's provider (`JELLYFIN`); notification provider types are the
 * plugin's lowercase type (`jellyfin`).
 */
export function mediaServerRefreshOffer(
  provider: string,
  installedProviders: readonly MediaServerRefreshProvider[],
): MediaServerRefreshOffer {
  const wanted = provider.trim().toLowerCase();
  const installed = installedProviders.find(
    (candidate) => candidate.providerType.trim().toLowerCase() === wanted,
  );
  if (!installed) {
    return { kind: "plugin_missing" };
  }
  const eventTypes = subscribableProviderNotificationEvents(
    MEDIA_SERVER_REFRESH_OFFER_EVENTS,
    installed.supportedEvents,
  );
  return eventTypes.length > 0
    ? { kind: "available", eventTypes }
    : { kind: "none" };
}

export type MediaServerRefreshSubscriptionFailure = {
  eventType: string;
  error: unknown;
};

/**
 * Subscribes a saved connection to each event in turn and returns the events
 * that could not be subscribed, so a partial set is reported rather than left
 * silently.
 */
export async function subscribeMediaServerRefreshes(
  connectionId: string,
  eventTypes: readonly string[],
  createSubscription: (input: {
    targetKind: "media_server_connection";
    targetId: string;
    eventType: string;
    scope: "global";
    scopeId: null;
    isEnabled: true;
  }) => Promise<void>,
): Promise<MediaServerRefreshSubscriptionFailure[]> {
  const failures: MediaServerRefreshSubscriptionFailure[] = [];
  for (const eventType of eventTypes) {
    try {
      await createSubscription({
        targetKind: "media_server_connection",
        targetId: connectionId,
        eventType,
        scope: "global",
        scopeId: null,
        isEnabled: true,
      });
    } catch (error) {
      failures.push({ eventType, error });
    }
  }
  return failures;
}
