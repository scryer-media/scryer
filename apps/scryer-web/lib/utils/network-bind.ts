/** Who can reach Scryer, as the setup question and the Security page ask it. */
export type NetworkReach = "local" | "network" | "custom";

/** The hosts the two plain choices save. */
export const LOCAL_HOST = "127.0.0.1";
export const NETWORK_HOST = "0.0.0.0";

/**
 * Codes the server reports when it is not listening where it was configured
 * to. They arrive in `bindFallbackCode`, never in an error, so they are
 * translated here; an unknown code shows the server's detail instead.
 */
export const NETWORK_FALLBACK_KEYS: Readonly<Record<string, string>> = {
  NETWORK_BIND_FALLBACK_UNBINDABLE: "settings.networkFallback.unbindable",
  NETWORK_BIND_STORED_HOST_INVALID: "settings.networkFallback.storedInvalid",
  NETWORK_BIND_STORED_HOST_UNREADABLE:
    "settings.networkFallback.storedUnreadable",
};

/** The host of a `host:port` or `[v6]:port` socket address. */
export function socketHost(address: string): string {
  const trimmed = address.trim();
  const bracketed = /^\[([^\]]+)\](?::\d+)?$/.exec(trimmed);
  if (bracketed) return bracketed[1];
  const separator = trimmed.lastIndexOf(":");
  // More than one colon without brackets is a bare IPv6 address.
  if (separator === -1 || trimmed.indexOf(":") !== separator) return trimmed;
  return trimmed.slice(0, separator);
}

function isLoopbackHost(host: string): boolean {
  const lowered = host.toLowerCase();
  return (
    /^127\./.test(lowered) ||
    lowered === "::1" ||
    lowered === "0:0:0:0:0:0:0:1" ||
    /^::ffff:127\./.test(lowered)
  );
}

/** Which choice a host is, so the page opens on what is configured. */
export function reachForHost(host: string): NetworkReach {
  if (host === LOCAL_HOST || isLoopbackHost(host)) return "local";
  if (host === NETWORK_HOST) return "network";
  return "custom";
}

/** The host to save for a choice, or null when the custom field is empty. */
export function hostForReach(
  reach: NetworkReach,
  customHost: string,
): string | null {
  if (reach === "local") return LOCAL_HOST;
  if (reach === "network") return NETWORK_HOST;
  const trimmed = customHost.trim();
  return trimmed === "" ? null : trimmed;
}
