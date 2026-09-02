/**
 * Proxies are a first-class integration of their own: indexers and download
 * clients both assign one, so the types live here rather than under indexers.
 */

/**
 * Every proxy provider the API accepts, in the order the editor lists them:
 * challenge solvers, then standard transport proxies, then tunnels.
 */
export const PROXY_PROVIDER_TYPES = [
  "byparr",
  "trawl",
  "http",
  "socks4",
  "socks5",
  "ssh_tunnel",
  "wireguard",
] as const;

export type ProxyProviderTypeValue = (typeof PROXY_PROVIDER_TYPES)[number];

/**
 * The three families a proxy can belong to. `tunnel` is deliberately a family
 * rather than "SSH", so a second tunnel technology slots in beside `ssh_tunnel`
 * without moving anything.
 */
export const PROXY_FAMILIES = ["solver", "standard", "tunnel"] as const;

export type ProxyFamily = (typeof PROXY_FAMILIES)[number];

const PROXY_PROVIDER_FAMILY: Record<ProxyProviderTypeValue, ProxyFamily> = {
  byparr: "solver",
  trawl: "solver",
  http: "standard",
  socks4: "standard",
  socks5: "standard",
  ssh_tunnel: "tunnel",
  wireguard: "tunnel",
};

export const PROXY_PROVIDER_TYPES_BY_FAMILY: Record<
  ProxyFamily,
  readonly ProxyProviderTypeValue[]
> = {
  solver: PROXY_PROVIDER_TYPES.filter(
    (providerType) => PROXY_PROVIDER_FAMILY[providerType] === "solver",
  ),
  standard: PROXY_PROVIDER_TYPES.filter(
    (providerType) => PROXY_PROVIDER_FAMILY[providerType] === "standard",
  ),
  tunnel: PROXY_PROVIDER_TYPES.filter(
    (providerType) => PROXY_PROVIDER_FAMILY[providerType] === "tunnel",
  ),
};

/**
 * Product and protocol names, so they stay identical in every locale. An
 * unknown value from a newer server is rendered verbatim rather than
 * mislabelled as something it is not.
 */
const PROXY_PROVIDER_LABELS: Record<ProxyProviderTypeValue, string> = {
  byparr: "Byparr",
  trawl: "Trawl",
  http: "HTTP",
  socks4: "SOCKS4",
  socks5: "SOCKS5",
  ssh_tunnel: "SSH tunnel",
  wireguard: "WireGuard",
};

/** Locale keys for the three family headings. */
export const PROXY_FAMILY_LABEL_KEYS: Record<ProxyFamily, string> = {
  solver: "settings.proxyFamilySolver",
  standard: "settings.proxyFamilyStandard",
  tunnel: "settings.proxyFamilyTunnel",
};

export function isProxyProviderType(
  value: string,
): value is ProxyProviderTypeValue {
  return (PROXY_PROVIDER_TYPES as readonly string[]).includes(value);
}

export function formatProxyProvider(providerType: string): string {
  return isProxyProviderType(providerType)
    ? PROXY_PROVIDER_LABELS[providerType]
    : providerType;
}

/** The family a provider belongs to, or null for a value this client does not know. */
export function proxyProviderFamily(providerType: string): ProxyFamily | null {
  return isProxyProviderType(providerType)
    ? PROXY_PROVIDER_FAMILY[providerType]
    : null;
}

/**
 * Standard transport proxies carry traffic as an HTTP/SOCKS hop; challenge
 * solvers answer browser challenges; tunnels dial their own transport. The
 * three kinds take entirely different settings.
 */
export function isTransportProxyProvider(providerType: string): boolean {
  return proxyProviderFamily(providerType) === "standard";
}

export function isSolverProxyProvider(providerType: string): boolean {
  return proxyProviderFamily(providerType) === "solver";
}

export function isTunnelProxyProvider(providerType: string): boolean {
  return proxyProviderFamily(providerType) === "tunnel";
}

/** The SSH half of the tunnel family: credentials, a passphrase, a host key. */
export function isSshTunnelProxyProvider(providerType: string): boolean {
  return providerType === "ssh_tunnel";
}

/**
 * The WireGuard half of the tunnel family. Its settings are disjoint from the
 * SSH ones — the API rejects every SSH field on a WireGuard row and every
 * WireGuard field on anything else — so the editor branches on this rather than
 * on the family.
 */
export function isWireguardProxyProvider(providerType: string): boolean {
  return providerType === "wireguard";
}

/**
 * Which providers accept a username and password. Challenge solvers take none;
 * SOCKS4 is rejected too, because the HTTP client builds its SOCKS4 connector
 * without auth and a credential would be silently dropped on the wire. An SSH
 * tunnel takes them as SSH credentials, and its username is mandatory —
 * WireGuard authenticates with keys and rejects both outright.
 */
export function supportsProxyCredentials(providerType: string): boolean {
  return (
    providerType === "http" ||
    providerType === "socks5" ||
    isSshTunnelProxyProvider(providerType)
  );
}

/**
 * Remote DNS is the `socks4a` / `socks5h` behaviour, so it is a SOCKS-only
 * choice. An HTTP CONNECT proxy always resolves the destination itself, a
 * solver fetches the page entirely on its own side, and a tunnel always
 * resolves on the far side.
 */
export function supportsProxyRemoteDns(providerType: string): boolean {
  return providerType === "socks4" || providerType === "socks5";
}

/**
 * Only tunnels carry key material; every other provider rejects it. Both
 * tunnel kinds take a private key, in their own encoding: PEM for SSH, base64
 * for WireGuard.
 */
export function supportsProxyPrivateKey(providerType: string): boolean {
  return isTunnelProxyProvider(providerType);
}

/**
 * A passphrase protects an OpenSSH key file. A WireGuard key is 32 raw bytes
 * with nothing to unlock, and the API rejects a passphrase on one.
 */
export function supportsProxyPrivateKeyPassphrase(
  providerType: string,
): boolean {
  return isSshTunnelProxyProvider(providerType);
}

/**
 * Host keys are the SSH trust-on-first-use step. WireGuard authenticates the
 * peer with the public key the operator already typed, so there is nothing to
 * pin and nothing to reset.
 */
export function supportsProxyHostKey(providerType: string): boolean {
  return isSshTunnelProxyProvider(providerType);
}

/**
 * The peer public key, preshared key, addresses, DNS servers, MTU and
 * keepalive. The API refuses all six on any other provider rather than
 * ignoring them, so they must never leave the client for one.
 */
export function supportsProxyWireguardFields(providerType: string): boolean {
  return isWireguardProxyProvider(providerType);
}

/**
 * WireGuard's own MTU bounds, mirrored from `scryer-tunnel`'s spec so the form
 * refuses a value the workflow would reject on the round trip.
 */
export const WIREGUARD_MTU_MIN = 1280;
export const WIREGUARD_MTU_MAX = 3800;
export const WIREGUARD_MTU_DEFAULT = 1280;

/** The engine's persistent-keepalive default, in seconds; `0` switches it off. */
export const WIREGUARD_KEEPALIVE_DEFAULT_SECONDS = 25;

/**
 * A WireGuard key as `wg genkey` prints it: 32 bytes of standard base64, which
 * is always 44 characters ending in `=`. This is a cheap shape check for the
 * form's own message only — the backend parses the key and is the authority on
 * whether it is usable.
 */
export function looksLikeWireguardKey(value: string): boolean {
  return /^[A-Za-z0-9+/]{43}=$/.test(value.trim());
}

/**
 * The attribute names a WireGuard configuration uses for each field, so a
 * pasted `Key = value` line is worth exactly as much as the bare value. These
 * mirror the workflow's own lists, which are the authority.
 */
export const ENDPOINT_CONFIG_KEYS = ["endpoint"] as const;
export const PRIVATE_KEY_CONFIG_KEYS = ["privatekey"] as const;
export const PEER_PUBLIC_KEY_CONFIG_KEYS = ["publickey", "peerpublickey"] as const;
export const PRESHARED_KEY_CONFIG_KEYS = ["presharedkey"] as const;
export const MTU_CONFIG_KEYS = ["mtu"] as const;
export const KEEPALIVE_CONFIG_KEYS = ["persistentkeepalive", "keepalive"] as const;
/**
 * Both list fields answer to either name: a stray `DNS =` pasted into the
 * address box is still an operator pasting a line, and stripping it is kinder
 * than refusing it.
 */
export const TUNNEL_LIST_CONFIG_KEYS = ["address", "addresses", "dns"] as const;

/**
 * Everything from an unquoted `#` or `;` is a comment in a `wg` file.
 *
 * Only applied to single-line values that cannot legitimately contain one: an
 * endpoint, a base64 key, an address list. A password can contain anything, so
 * it is deliberately never run through this.
 */
export function stripTrailingComment(line: string): string {
  for (let index = 0; index < line.length; index += 1) {
    const character = line[index];
    if (
      (character === "#" || character === ";") &&
      (index === 0 || /\s/.test(line[index - 1] ?? ""))
    ) {
      return line.slice(0, index).trimEnd();
    }
  }
  return line;
}

/**
 * Take the value out of a `Key = value` line when the key is one this field
 * answers to, and hand back anything else untouched.
 *
 * Matching the key by name is what makes this safe: a base64 key ends in `=` as
 * well, but `xJ4…` is not an attribute name, so such a value is kept whole. A
 * multi-line value is a PEM block rather than an assignment, and is returned as
 * it arrived.
 */
export function stripConfigAssignment(
  raw: string,
  keys: readonly string[],
): string {
  const trimmed = raw.trim();
  if (trimmed.includes("\n")) {
    return trimmed;
  }
  const value = stripTrailingComment(trimmed);
  const separator = value.indexOf("=");
  if (separator === -1) {
    return value;
  }
  const name = value.slice(0, separator).trim().toLowerCase();
  return keys.includes(name) ? value.slice(separator + 1).trim() : value;
}

/** The URL scheme each provider's endpoint has to carry. */
const PROXY_URL_SCHEMES: Record<ProxyProviderTypeValue, string> = {
  byparr: "http",
  trawl: "http",
  http: "http",
  socks4: "socks4",
  socks5: "socks5",
  ssh_tunnel: "ssh",
  wireguard: "wireguard",
};

function hasUrlScheme(value: string): boolean {
  return /^[A-Za-z][A-Za-z0-9+.-]*:\/\//.test(value);
}

/**
 * The endpoint as it will be stored: the assignment stripped, and the
 * provider's own scheme supplied when the operator pasted a bare authority.
 *
 * `vpn.example.com:51820` is what a WireGuard configuration contains and what
 * an operator naturally types. A bare IPv6 literal is bracketed on the way
 * through, because `fd00::1` is a host and only `[fd00::1]:51820` can also
 * carry a port. A scheme the operator actually wrote is never rewritten: that
 * would hide a real mistake rather than forgive a paste.
 */
export function normalizeProxyEndpoint(
  providerType: string,
  raw: string,
): string {
  const stripped = stripConfigAssignment(raw, ENDPOINT_CONFIG_KEYS).replace(
    /\/+$/,
    "",
  );
  if (stripped === "" || hasUrlScheme(stripped)) {
    return stripped;
  }
  if (!isProxyProviderType(providerType)) {
    // A provider only a newer server knows: no scheme of ours to supply.
    return stripped;
  }
  const scheme = PROXY_URL_SCHEMES[providerType];
  const authority =
    !stripped.startsWith("[") && stripped.split(":").length > 2
      ? `[${stripped}]`
      : stripped;
  return `${scheme}://${authority}`;
}

/**
 * Split an address or DNS list the way the workflow does: a `wg` config writes
 * them as one comma-separated line, and an operator is at least as likely to
 * paste one entry per line or the whole `Address = …` line, so all of those
 * mean the same list. Blank entries and comments are dropped, so a trailing
 * newline or comma costs nothing.
 */
export function splitTunnelList(raw: string): string[] {
  return raw
    .split("\n")
    .map((line) => stripConfigAssignment(line, TUNNEL_LIST_CONFIG_KEYS))
    .flatMap((line) => line.split(","))
    .map((entry) => entry.trim())
    .filter((entry) => entry !== "");
}

/** The stored list as the textarea shows it: one entry per line. */
export function formatTunnelList(values: readonly string[]): string {
  return values.join("\n");
}

/**
 * Group rows for a family-ordered list or dropdown. Providers this client does
 * not recognise are collected under `family: null` so they are still shown,
 * never dropped.
 */
export function groupProxiesByFamily<T extends { providerType: string }>(
  proxies: readonly T[],
): Array<{ family: ProxyFamily | null; proxies: T[] }> {
  const groups: Array<{ family: ProxyFamily | null; proxies: T[] }> = [
    ...PROXY_FAMILIES.map((family) => ({
      family: family as ProxyFamily | null,
      proxies: [] as T[],
    })),
    { family: null, proxies: [] as T[] },
  ];

  for (const proxy of proxies) {
    const family = proxyProviderFamily(proxy.providerType);
    const group = groups.find((candidate) => candidate.family === family);
    group?.proxies.push(proxy);
  }

  return groups.filter((group) => group.proxies.length > 0);
}

export type ProxyRecord = {
  id: string;
  name: string;
  providerType: string;
  /** Null for transport proxies and tunnels, which speak no solver protocol. */
  protocol: string | null;
  baseUrl: string;
  requestTimeoutSeconds: number;
  /** Whether a username or password is stored, never the values themselves. */
  hasCredentials: boolean;
  /** SOCKS only: destination hostnames are resolved at the proxy. */
  remoteDns: boolean;
  /** Whether a tunnel private key is stored, never the key itself. */
  hasPrivateKey: boolean;
  /**
   * WireGuard's `[Peer]` public key, or null outside WireGuard. A public key
   * is public, so it is read back in full rather than masked.
   */
  peerPublicKey: string | null;
  /** Whether a WireGuard preshared key is stored, never the key itself. */
  hasPresharedKey: boolean;
  /**
   * This tunnel's own public key, derived from its private key, or null while
   * no key is stored. It is the line the operator pastes into the server's
   * `[Peer]` section, so it is shown rather than masked.
   */
  tunnelPublicKey: string | null;
  /** WireGuard `[Interface] Address` entries; empty outside WireGuard. */
  tunnelAddresses: string[];
  /** WireGuard `[Interface] DNS` entries; empty when none are configured. */
  tunnelDnsServers: string[];
  /** WireGuard tunnel MTU, or null to use the engine's default. */
  tunnelMtu: number | null;
  /**
   * WireGuard persistent keepalive in seconds, or null for the engine's
   * default. Zero means keepalive is switched off.
   */
  tunnelKeepaliveSeconds: number | null;
  /**
   * Host key pinned on the first successful tunnel connect, as OpenSSH prints
   * it (`SHA256:<base64>`), or null before the first connect.
   */
  hostKeyFingerprint: string | null;
  /** UTC time the host key above was pinned, or null when none is pinned. */
  hostKeyPinnedAt: string | null;
  isEnabled: boolean;
  lastHealthStatus: string | null;
  lastErrorMessage: string | null;
  lastErrorAt: string | null;
  createdAt: string;
  updatedAt: string;
};

export type ProxyDraft = {
  providerType: ProxyProviderTypeValue;
  name: string;
  /** Solver/standard base URL, or a tunnel's `ssh://host:port` endpoint. */
  baseUrl: string;
  requestTimeoutSeconds: number;
  /**
   * Write-only credentials. Blank on an edit means "leave the stored secret
   * alone" — they are never read back from the API.
   */
  username: string;
  password: string;
  /** Whether the proxy being edited already has a stored credential. */
  hasStoredCredentials: boolean;
  /** Standard proxies: drop the stored username and password outright. */
  clearCredentials: boolean;
  /**
   * Tunnels: drop the stored password alone. A tunnel's username is mandatory,
   * so it can never be cleared the way a standard proxy's pair can.
   */
  clearPassword: boolean;
  /** Write-only PEM private key for a tunnel; blank means unchanged. */
  privateKey: string;
  privateKeyPassphrase: string;
  /** Whether the tunnel being edited already has a stored private key. */
  hasStoredPrivateKey: boolean;
  /** Tunnels: drop the stored private key and its passphrase. */
  clearPrivateKey: boolean;
  /**
   * WireGuard's `[Peer]` public key. Not a secret, so it is read back into the
   * draft in full; blank on an update means "leave the stored value alone".
   */
  peerPublicKey: string;
  /** Write-only WireGuard preshared key; blank means unchanged. */
  presharedKey: string;
  /** Whether the WireGuard row being edited already has a preshared key. */
  hasStoredPresharedKey: boolean;
  /** WireGuard: drop the stored preshared key. */
  clearPresharedKey: boolean;
  /**
   * WireGuard `[Interface] Address` entries exactly as pasted — one per line
   * or comma-separated — split on the way out.
   */
  tunnelAddresses: string;
  /** WireGuard `[Interface] DNS` entries, same free-text shape. */
  tunnelDnsServers: string;
  /** WireGuard MTU as typed; blank means the engine's default. */
  tunnelMtu: string;
  /**
   * Whether the row being edited stores an MTU. Blanking the field only
   * restores the default when there was something to restore it from;
   * otherwise the field is omitted rather than sent as an explicit null.
   */
  hasStoredTunnelMtu: boolean;
  /** WireGuard keepalive seconds as typed; blank means the default, `0` off. */
  tunnelKeepaliveSeconds: string;
  /** Whether the row being edited stores a keepalive. Same rule as the MTU. */
  hasStoredTunnelKeepaliveSeconds: boolean;
  /** SOCKS only: resolve destination hostnames at the proxy (`socks5h`). */
  remoteDns: boolean;
  isEnabled: boolean;
};

/**
 * Each provider's endpoint has to match its own scheme, so switching provider
 * in the editor reseeds the placeholder rather than leaving a solver URL on a
 * SOCKS row.
 */
export const PROXY_DEFAULT_BASE_URLS: Record<ProxyProviderTypeValue, string> = {
  byparr: "http://localhost:8191",
  trawl: "http://localhost:8191",
  http: "http://localhost:3128",
  socks4: "socks4://localhost:1080",
  socks5: "socks5://localhost:1080",
  ssh_tunnel: "ssh://localhost:22",
  wireguard: "wireguard://localhost:51820",
};

export const PROXY_INITIAL_DRAFT: ProxyDraft = {
  providerType: "byparr",
  name: "",
  baseUrl: PROXY_DEFAULT_BASE_URLS.byparr,
  requestTimeoutSeconds: 60,
  username: "",
  password: "",
  hasStoredCredentials: false,
  clearCredentials: false,
  clearPassword: false,
  privateKey: "",
  privateKeyPassphrase: "",
  hasStoredPrivateKey: false,
  clearPrivateKey: false,
  peerPublicKey: "",
  presharedKey: "",
  hasStoredPresharedKey: false,
  clearPresharedKey: false,
  tunnelAddresses: "",
  tunnelDnsServers: "",
  tunnelMtu: "",
  hasStoredTunnelMtu: false,
  tunnelKeepaliveSeconds: "",
  hasStoredTunnelKeepaliveSeconds: false,
  remoteDns: false,
  isEnabled: true,
};

/**
 * The draft as it will be stored: every field cleaned of the `Key = value`
 * shape an operator pastes, the endpoint carrying its provider's scheme, and
 * the two lists rewritten one entry per line.
 *
 * Applied on save rather than on every keystroke, so nothing fights the
 * operator while they type, and the form then shows exactly what was stored.
 * The workflow normalizes the same way and stays the authority; doing it here
 * as well is what makes the form's own validation see the same values the API
 * will.
 */
export function normalizeProxyDraft(draft: ProxyDraft): ProxyDraft {
  const baseUrl = normalizeProxyEndpoint(draft.providerType, draft.baseUrl);
  if (!supportsProxyWireguardFields(draft.providerType)) {
    return baseUrl === draft.baseUrl ? draft : { ...draft, baseUrl };
  }
  return {
    ...draft,
    baseUrl,
    privateKey: stripConfigAssignment(draft.privateKey, PRIVATE_KEY_CONFIG_KEYS),
    peerPublicKey: stripConfigAssignment(
      draft.peerPublicKey,
      PEER_PUBLIC_KEY_CONFIG_KEYS,
    ),
    presharedKey: stripConfigAssignment(
      draft.presharedKey,
      PRESHARED_KEY_CONFIG_KEYS,
    ),
    tunnelAddresses: formatTunnelList(splitTunnelList(draft.tunnelAddresses)),
    tunnelDnsServers: formatTunnelList(splitTunnelList(draft.tunnelDnsServers)),
    tunnelMtu: stripConfigAssignment(draft.tunnelMtu, MTU_CONFIG_KEYS),
    tunnelKeepaliveSeconds: stripConfigAssignment(
      draft.tunnelKeepaliveSeconds,
      KEEPALIVE_CONFIG_KEYS,
    ),
  };
}
