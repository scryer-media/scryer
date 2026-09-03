export type JellyfinMediaServerConnection = {
  displayName: string;
  enabled: boolean;
  linkingEnabled: boolean;
  apiKeyPresent: boolean;
  externalUrl: string | null;
};

export type OAuthClientKind = "CUSTOM" | "JELLYFIN_PLUGIN";

export type OAuthClientRegistrationForJellyfin = {
  clientId: string;
  redirectUris: string[];
  enabled: boolean;
  source: "MANAGED" | "CUSTOM";
  /** Stored at registration and immutable afterwards. */
  kind: OAuthClientKind;
};

export type JellyfinPluginClient = {
  clientId: string;
  callbackUrl: string;
};

/** Why a Jellyfin connection cannot back the `jellyfin-link` scope, worded as the backend words it. */
export type JellyfinConnectionIneligibility = {
  displayName: string;
  reason: string;
};

export const JELLYFIN_PLUGIN_CALLBACK_PATH = "/Scryer/Auth/Callback";

export function normalizedPublicJellyfinBaseUrl(value: string): string | null {
  try {
    const url = new URL(value.trim());
    if (
      url.protocol !== "https:" ||
      !url.hostname ||
      url.username ||
      url.password ||
      url.search ||
      url.hash
    ) return null;
    return url.toString().replace(/\/+$/, "");
  } catch {
    return null;
  }
}

export function eligibleJellyfinConnectionBaseUrls(
  connections: JellyfinMediaServerConnection[],
): string[] {
  return connections.flatMap((connection) => {
    const externalUrl = normalizedPublicJellyfinBaseUrl(connection.externalUrl ?? "");
    return (
      connection.enabled
      && connection.linkingEnabled
      && connection.apiKeyPresent
      && externalUrl
    ) ? [externalUrl] : [];
  });
}

/** Every Jellyfin connection with a usable public HTTPS URL, eligible for linking or not. */
export function jellyfinConnectionBaseUrls(
  connections: JellyfinMediaServerConnection[],
): string[] {
  return connections.flatMap((connection) => {
    const externalUrl = normalizedPublicJellyfinBaseUrl(connection.externalUrl ?? "");
    return externalUrl ? [externalUrl] : [];
  });
}

/**
 * Prefills from any Jellyfin connection with a public HTTPS URL. A disabled or unlinked
 * connection still prefills so the operator sees the ineligibility warning here instead of
 * discovering it on the Authorize page.
 */
export function prefillJellyfinPublicBaseUrl(
  connections: JellyfinMediaServerConnection[] | null,
): string | null {
  if (!connections) return null;
  const baseUrls = jellyfinConnectionBaseUrls(connections);
  return baseUrls.length === 1 ? baseUrls[0] : null;
}

/** The first failing condition, in the backend's order, for connections on this public URL. */
export function jellyfinConnectionIneligibilityReasons(
  publicBaseUrl: string | null,
  connections: JellyfinMediaServerConnection[] | null,
): JellyfinConnectionIneligibility[] {
  if (!publicBaseUrl || !connections) return [];
  return connections.flatMap((connection) => {
    if (normalizedPublicJellyfinBaseUrl(connection.externalUrl ?? "") !== publicBaseUrl) return [];
    const reason = !connection.enabled
      ? "is disabled"
      : !connection.linkingEnabled
        ? "has account linking disabled"
        : !connection.apiKeyPresent
          ? "has no API key"
          : null;
    return reason ? [{ displayName: connection.displayName, reason }] : [];
  });
}

export function automaticLinkingStatusForConnections(
  publicBaseUrl: string,
  connections: JellyfinMediaServerConnection[],
): "not-ready" | "ambiguous" | "ready" {
  const matchingConnectionCount = eligibleJellyfinConnectionBaseUrls(connections).filter(
    (externalUrl) => externalUrl === publicBaseUrl,
  ).length;
  if (matchingConnectionCount === 1) return "ready";
  return matchingConnectionCount > 1 ? "ambiguous" : "not-ready";
}

export function automaticLinkingStatus(
  publicBaseUrl: string | null,
  connections: JellyfinMediaServerConnection[] | null,
): "unavailable" | "enter-url" | "not-ready" | "ambiguous" | "ready" {
  if (connections === null) return "unavailable";
  if (!publicBaseUrl) return "enter-url";
  return automaticLinkingStatusForConnections(publicBaseUrl, connections);
}

export function jellyfinPluginCallbackUrl(publicBaseUrl: string | null): string | null {
  return publicBaseUrl ? `${publicBaseUrl}${JELLYFIN_PLUGIN_CALLBACK_PATH}` : null;
}

/**
 * Recovers, for display only, the public Jellyfin base URL a stored callback was derived from.
 * A callback that does not decompose this way is still a perfectly valid callback.
 */
export function jellyfinPublicBaseUrlFromCallback(callbackUrl: string): string | null {
  if (!callbackUrl.endsWith(JELLYFIN_PLUGIN_CALLBACK_PATH)) return null;
  return normalizedPublicJellyfinBaseUrl(
    callbackUrl.slice(0, callbackUrl.length - JELLYFIN_PLUGIN_CALLBACK_PATH.length),
  );
}

/**
 * The plugin client is identified by the kind stored on its registration, never by the shape of
 * its callback URL. The server applies the same rule when it decides whether `jellyfin-link` can
 * bind, so the panel and the authorization page always agree on which client is the plugin.
 */
export function isJellyfinPluginClientRegistration(
  registeredClient: OAuthClientRegistrationForJellyfin,
): boolean {
  return registeredClient.kind === "JELLYFIN_PLUGIN";
}

/**
 * Plugin clients with their callback and public base URL pulled out for display. Both are derived
 * from stored data and may be absent; neither takes part in identifying the client.
 */
export function jellyfinPluginClientRegistrations<
  TClient extends OAuthClientRegistrationForJellyfin,
>(clients: TClient[]): Array<TClient & { callbackUrl: string | null; publicBaseUrl: string | null }> {
  return clients
    .filter((registeredClient) => isJellyfinPluginClientRegistration(registeredClient))
    .map((registeredClient) => {
      const callbackUrl = registeredClient.redirectUris[0] ?? null;
      return {
        ...registeredClient,
        callbackUrl,
        publicBaseUrl: callbackUrl ? jellyfinPublicBaseUrlFromCallback(callbackUrl) : null,
      };
    });
}

/** A plugin client that can actually serve this callback today. */
export function isEligibleJellyfinPluginClient(
  registeredClient: OAuthClientRegistrationForJellyfin,
  callbackUrl: string,
): boolean {
  return (
    isJellyfinPluginClientRegistration(registeredClient)
    && registeredClient.enabled
    && registeredClient.redirectUris.includes(callbackUrl)
  );
}

export function jellyfinPluginClientCreateDecision(
  clients: OAuthClientRegistrationForJellyfin[],
  callbackUrl: string,
): "create" | "reuse" | "ambiguous" {
  const matches = clients.filter((client) => isEligibleJellyfinPluginClient(client, callbackUrl));
  if (matches.length > 1) return "ambiguous";
  return matches.length === 1 ? "reuse" : "create";
}

export function reconcileCreatedJellyfinPluginClient(
  candidate: JellyfinPluginClient | null,
  clients: OAuthClientRegistrationForJellyfin[],
): JellyfinPluginClient | null {
  if (!candidate) return null;
  return clients.some((client) =>
    client.clientId === candidate.clientId
    && isEligibleJellyfinPluginClient(client, candidate.callbackUrl),
  ) ? candidate : null;
}

export function createdJellyfinPluginClientForCallback(
  candidate: JellyfinPluginClient | null,
  callbackUrl: string | null,
): JellyfinPluginClient | null {
  return candidate?.callbackUrl === callbackUrl ? candidate : null;
}

/** A response from an older reload must never replace newer settings data. */
export function shouldApplyJellyfinPluginOAuthReload(
  responseGeneration: number,
  currentGeneration: number,
): boolean {
  return responseGeneration === currentGeneration;
}

/** The panel has one mutation slot, so a second click cannot create a duplicate client. */
export function canStartJellyfinPluginClientCreation(inFlightCallbackUrl: string | null): boolean {
  return inFlightCallbackUrl === null;
}

export function canCreateJellyfinPluginClient(
  busy: boolean,
  status: "ambiguous" | "ready" | "reconciling" | "not-configured",
): boolean {
  return !busy && status !== "ambiguous" && status !== "reconciling";
}

export function jellyfinPluginClientStatus(
  matchingClientCount: number,
  createdClientMatchesCallback: boolean,
  uncertainCreateMatchesCallback: boolean,
): "ambiguous" | "ready" | "reconciling" | "not-configured" {
  if (matchingClientCount > 1) return "ambiguous";
  if (matchingClientCount === 1) return "ready";
  return createdClientMatchesCallback || uncertainCreateMatchesCallback
    ? "reconciling"
    : "not-configured";
}

export function jellyfinPluginCreateNeedsReconciliation(
  registration: { clientId: string } | null | undefined,
): registration is null | undefined {
  return !registration;
}
