export type JellyfinMediaServerConnection = {
  enabled: boolean;
  linkingEnabled: boolean;
  apiKeyPresent: boolean;
  externalUrl: string | null;
};

export type OAuthClientRegistrationForJellyfin = {
  clientId: string;
  redirectUris: string[];
  enabled: boolean;
  source: "MANAGED" | "CUSTOM";
};

export type JellyfinPluginClient = {
  clientId: string;
  callbackUrl: string;
};

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

export function prefillJellyfinPublicBaseUrl(
  connections: JellyfinMediaServerConnection[] | null,
): string | null {
  if (!connections) return null;
  const eligibleBaseUrls = eligibleJellyfinConnectionBaseUrls(connections);
  return eligibleBaseUrls.length === 1 ? eligibleBaseUrls[0] : null;
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
  return publicBaseUrl ? `${publicBaseUrl}/Scryer/Auth/Callback` : null;
}

export function isEligibleJellyfinPluginClient(
  registeredClient: OAuthClientRegistrationForJellyfin,
  callbackUrl: string,
): boolean {
  return (
    registeredClient.source === "CUSTOM"
    && registeredClient.enabled
    && registeredClient.redirectUris.length === 1
    && registeredClient.redirectUris[0] === callbackUrl
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
