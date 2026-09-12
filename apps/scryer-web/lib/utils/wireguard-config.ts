import { stripTrailingComment } from "../types/proxies.ts";

/**
 * What one `wg-quick` configuration says, in the shape of the proxy draft's
 * own fields so the editor can fill itself from it.
 *
 * An operator is handed a `.conf` file by their VPN provider or their own
 * server. Retyping eight fields out of it is exactly the kind of transcription
 * that produces a key with a missing character and an evening of debugging, so
 * the file is read here instead.
 */
export type WireguardConfigImport = {
  /** `[Peer] Endpoint`, as written: the scheme is supplied when it is stored. */
  endpoint: string;
  /** `[Interface] PrivateKey`. */
  privateKey: string;
  /** `[Peer] PublicKey`. */
  peerPublicKey: string;
  /** `[Peer] PresharedKey`, when the server uses one. */
  presharedKey: string;
  /** `[Interface] Address`, one entry per line. */
  tunnelAddresses: string;
  /** `[Interface] DNS`, one entry per line. */
  tunnelDnsServers: string;
  /** `[Interface] MTU`. */
  tunnelMtu: string;
  /** `[Peer] PersistentKeepalive`. */
  tunnelKeepaliveSeconds: string;
  /**
   * Keys that are WireGuard's but not a Scryer tunnel's, in the order they
   * appeared. Reported rather than silently dropped: an operator who wrote
   * `AllowedIPs = 10.0.0.0/24` deserves to know it did not survive.
   */
  ignored: string[];
  /** How many `[Peer]` sections the file had. Only the first is imported. */
  peerCount: number;
};

/**
 * Keys a `wg` configuration legitimately carries that a Scryer tunnel has no
 * use for.
 *
 * `AllowedIPs` is the interesting one: this tunnel is only ever dialled *into*,
 * so the engine routes every destination to the peer and the file's own value
 * cannot narrow that. `ListenPort` is ours to choose because nothing dials us,
 * and the script hooks belong to `wg-quick`, which is not running here.
 */
const IGNORED_CONFIG_KEYS = new Set([
  "allowedips",
  "listenport",
  "table",
  "fwmark",
  "preup",
  "postup",
  "predown",
  "postdown",
  "saveconfig",
]);

const EMPTY_IMPORT: WireguardConfigImport = {
  endpoint: "",
  privateKey: "",
  peerPublicKey: "",
  presharedKey: "",
  tunnelAddresses: "",
  tunnelDnsServers: "",
  tunnelMtu: "",
  tunnelKeepaliveSeconds: "",
  ignored: [],
  peerCount: 0,
};

/**
 * Read a pasted or uploaded `wg-quick` configuration.
 *
 * Deliberately tolerant: keys are resolved by name rather than by section, so
 * a fragment with no `[Interface]` header reads as well as a whole file, and a
 * section header is only consulted to know which `[Peer]` we are inside.
 * Returns `null` when nothing in the text was recognisable, which is how the
 * caller tells "not a WireGuard configuration" from "a sparse one".
 */
export function parseWireguardConfig(text: string): WireguardConfigImport | null {
  const result: WireguardConfigImport = { ...EMPTY_IMPORT, ignored: [] };
  const addresses: string[] = [];
  const dnsServers: string[] = [];
  const ignored = new Set<string>();
  let recognised = false;
  let inPeer = false;

  for (const rawLine of text.split(/\r?\n/)) {
    const line = stripTrailingComment(rawLine).trim();
    if (line === "") {
      continue;
    }
    const section = /^\[\s*([A-Za-z]+)\s*\]$/.exec(line);
    if (section) {
      inPeer = (section[1] ?? "").toLowerCase() === "peer";
      if (inPeer) {
        result.peerCount += 1;
      }
      continue;
    }
    const separator = line.indexOf("=");
    if (separator === -1) {
      continue;
    }
    const key = line.slice(0, separator).trim().toLowerCase();
    const value = line.slice(separator + 1).trim();
    if (value === "") {
      continue;
    }
    if (IGNORED_CONFIG_KEYS.has(key)) {
      ignored.add(line.slice(0, separator).trim());
      continue;
    }
    // Only the first peer is imported: one proxy is one tunnel to one peer.
    const isLaterPeer = inPeer && result.peerCount > 1;
    switch (key) {
      case "privatekey":
        recognised = true;
        if (!isLaterPeer) result.privateKey ||= value;
        break;
      case "address":
        recognised = true;
        if (!isLaterPeer) addresses.push(value);
        break;
      case "dns":
        recognised = true;
        if (!isLaterPeer) dnsServers.push(value);
        break;
      case "mtu":
        recognised = true;
        if (!isLaterPeer) result.tunnelMtu ||= value;
        break;
      case "publickey":
        recognised = true;
        if (!isLaterPeer) result.peerPublicKey ||= value;
        break;
      case "presharedkey":
        recognised = true;
        if (!isLaterPeer) result.presharedKey ||= value;
        break;
      case "endpoint":
        recognised = true;
        if (!isLaterPeer) result.endpoint ||= value;
        break;
      case "persistentkeepalive":
        recognised = true;
        // `0` is a value: it switches keepalive off.
        if (!isLaterPeer && result.tunnelKeepaliveSeconds === "") {
          result.tunnelKeepaliveSeconds = value;
        }
        break;
      default:
        break;
    }
  }

  if (!recognised) {
    return null;
  }
  result.tunnelAddresses = joinConfigList(addresses);
  result.tunnelDnsServers = joinConfigList(dnsServers);
  result.ignored = [...ignored];
  return result;
}

/**
 * One entry per line, from however many comma-separated lines the file used.
 * A repeated `Address =` and a single comma-separated one mean the same list.
 */
function joinConfigList(values: readonly string[]): string {
  return values
    .flatMap((value) => value.split(","))
    .map((entry) => entry.trim())
    .filter((entry) => entry !== "")
    .join("\n");
}
