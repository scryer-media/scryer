import type {
  ProxyDraft,
  UiDateTimeFormat,
  VerificationDepth,
} from "../types/index.ts";
// Imported from the module rather than the barrel: these are runtime values,
// and the barrel's extensionless re-exports only resolve for erased types.
import {
  isTunnelProxyProvider,
  splitTunnelList,
  supportsProxyCredentials,
  supportsProxyPrivateKey,
  supportsProxyPrivateKeyPassphrase,
  supportsProxyRemoteDns,
  supportsProxyWireguardFields,
} from "../types/proxies.ts";

function buildProxyCommonInput(draft: ProxyDraft) {
  return {
    name: draft.name.trim(),
    baseUrl: draft.baseUrl.trim(),
    requestTimeoutSeconds: draft.requestTimeoutSeconds,
    isEnabled: draft.isEnabled,
  };
}

/**
 * Remote DNS is a SOCKS-only setting: the API rejects `true` on anything else,
 * so the field never leaves the client for another provider.
 */
function buildProxyRemoteDnsInput(draft: ProxyDraft) {
  return supportsProxyRemoteDns(draft.providerType)
    ? { remoteDns: draft.remoteDns }
    : {};
}

/**
 * Credentials are write-only, the same convention as the indexer API key: an
 * omitted field keeps whatever is stored, an explicit null clears it, and a
 * value replaces it. Challenge solvers and SOCKS4 take no credentials at all.
 *
 * A tunnel's username is mandatory, so a tunnel can only ever drop its
 * password; clearing the pair the way a standard proxy does would leave a
 * configuration the API rejects.
 */
function buildProxyCredentialInput(
  draft: ProxyDraft,
  { allowClear }: { allowClear: boolean },
): { username?: string | null; password?: string | null } {
  if (!supportsProxyCredentials(draft.providerType)) {
    return {};
  }
  const isTunnel = isTunnelProxyProvider(draft.providerType);
  const username = draft.username.trim();
  const password = draft.password.trim();

  if (allowClear && !isTunnel && draft.clearCredentials) {
    return { username: null, password: null };
  }
  if (allowClear && isTunnel && draft.clearPassword) {
    return { ...(username ? { username } : {}), password: null };
  }

  return {
    ...(username ? { username } : {}),
    ...(password ? { password } : {}),
  };
}

/**
 * Tunnel key material, same write-only tri-state as the credentials. A
 * passphrase only means anything alongside a key, so it is withheld until one
 * is either pasted now or already stored — the API rejects it otherwise.
 */
function buildProxyPrivateKeyInput(
  draft: ProxyDraft,
  { allowClear }: { allowClear: boolean },
): { privateKey?: string | null; privateKeyPassphrase?: string | null } {
  if (!supportsProxyPrivateKey(draft.providerType)) {
    return {};
  }
  // A WireGuard key is 32 raw bytes with nothing to unlock, and the API
  // rejects a passphrase on one, so only an SSH tunnel ever carries the pair.
  const acceptsPassphrase = supportsProxyPrivateKeyPassphrase(
    draft.providerType,
  );
  if (allowClear && draft.clearPrivateKey) {
    // The passphrase protects the key, so it goes with it.
    return {
      privateKey: null,
      ...(acceptsPassphrase ? { privateKeyPassphrase: null } : {}),
    };
  }
  const privateKey = draft.privateKey.trim();
  const passphrase = draft.privateKeyPassphrase.trim();
  const hasKey = privateKey !== "" || draft.hasStoredPrivateKey;
  return {
    ...(privateKey ? { privateKey } : {}),
    ...(acceptsPassphrase && passphrase && hasKey
      ? { privateKeyPassphrase: passphrase }
      : {}),
  };
}

/**
 * WireGuard's own fields. Every other provider refuses all six outright rather
 * than ignoring them, so nothing here leaves the client for one.
 *
 * The two lists are the exception to the "omit means unchanged" convention the
 * secrets follow: the editor holds the whole list as text, so an update always
 * states it, and an empty list is how DNS servers are cleared. Addresses are
 * required, and the container refuses an empty one before this is reached.
 *
 * MTU and keepalive are tri-state in the other direction: a typed number sets
 * it, and blanking a value that was stored restores the engine's default with
 * an explicit null. Blanking a field that never had a value omits it, so a
 * create — where nothing is stored — never sends a null.
 */
function buildProxyWireguardInput(
  draft: ProxyDraft,
  { allowClear }: { allowClear: boolean },
): {
  peerPublicKey?: string;
  presharedKey?: string | null;
  tunnelAddresses?: string[];
  tunnelDnsServers?: string[];
  tunnelMtu?: number | null;
  tunnelKeepaliveSeconds?: number | null;
} {
  if (!supportsProxyWireguardFields(draft.providerType)) {
    return {};
  }
  const peerPublicKey = draft.peerPublicKey.trim();
  const presharedKey = draft.presharedKey.trim();
  const addresses = splitTunnelList(draft.tunnelAddresses);
  const dnsServers = splitTunnelList(draft.tunnelDnsServers);

  return {
    // The peer's key has no cleared state — a WireGuard tunnel cannot exist
    // without one — so an untouched field keeps whatever is stored.
    ...(peerPublicKey ? { peerPublicKey } : {}),
    ...buildProxyPresharedKeyInput(draft, presharedKey, { allowClear }),
    // Addresses are required, so they are always stated. DNS servers are not:
    // on a create there is nothing to clear, so an empty list is an omission.
    tunnelAddresses: addresses,
    ...(allowClear || dnsServers.length > 0
      ? { tunnelDnsServers: dnsServers }
      : {}),
    ...buildProxyTunnelNumberInput(
      "tunnelMtu",
      draft.tunnelMtu,
      draft.hasStoredTunnelMtu,
    ),
    ...buildProxyTunnelNumberInput(
      "tunnelKeepaliveSeconds",
      draft.tunnelKeepaliveSeconds,
      draft.hasStoredTunnelKeepaliveSeconds,
    ),
  };
}

function buildProxyPresharedKeyInput(
  draft: ProxyDraft,
  presharedKey: string,
  { allowClear }: { allowClear: boolean },
): { presharedKey?: string | null } {
  if (allowClear && draft.clearPresharedKey) {
    return { presharedKey: null };
  }
  return presharedKey ? { presharedKey } : {};
}

/**
 * One numeric WireGuard setting. Blank restores the engine's default, but only
 * when there was a stored value to restore it from: sending a null for a field
 * that was already unset would be a no-op write dressed as an intent.
 */
function buildProxyTunnelNumberInput<Field extends string>(
  field: Field,
  raw: string,
  hasStoredValue: boolean,
): Partial<Record<Field, number | null>> {
  const trimmed = raw.trim();
  if (trimmed === "") {
    return hasStoredValue
      ? ({ [field]: null } as Record<Field, null>)
      : ({} as Partial<Record<Field, number>>);
  }
  const parsed = Number.parseInt(trimmed, 10);
  return Number.isNaN(parsed)
    ? ({} as Partial<Record<Field, number>>)
    : ({ [field]: parsed } as Record<Field, number>);
}

export function buildCreateProxyInput(draft: ProxyDraft) {
  return {
    providerType: draft.providerType,
    ...buildProxyCommonInput(draft),
    ...buildProxyRemoteDnsInput(draft),
    // Nothing is stored yet, so there is nothing to clear.
    ...buildProxyCredentialInput(draft, { allowClear: false }),
    ...buildProxyPrivateKeyInput(draft, { allowClear: false }),
    ...buildProxyWireguardInput(draft, { allowClear: false }),
  };
}

export function buildUpdateProxyInput(
  id: string,
  draft: ProxyDraft,
) {
  return {
    id,
    ...buildProxyCommonInput(draft),
    ...buildProxyRemoteDnsInput(draft),
    ...buildProxyCredentialInput(draft, { allowClear: true }),
    ...buildProxyPrivateKeyInput(draft, { allowClear: true }),
    ...buildProxyWireguardInput(draft, { allowClear: true }),
  };
}

/**
 * A download client's proxy assignment on create. Nothing is stored yet, so
 * "no proxy" is an omission rather than a clear.
 */
export function buildDownloadClientProxyCreateInput(
  proxyConfigId: string | null,
): { proxyConfigId?: string } {
  return proxyConfigId ? { proxyConfigId } : {};
}

/**
 * A download client's proxy assignment on update. The editor always knows the
 * intended assignment, so it sends the value or an explicit null to clear it;
 * callers that are not editing the assignment (an enable toggle, a reorder)
 * omit the field entirely by not calling this, which preserves it.
 */
export function buildDownloadClientProxyUpdateInput(
  proxyConfigId: string | null,
): { proxyConfigId: string | null } {
  return { proxyConfigId };
}

export function parseUiDateTimeFormat(value: string): UiDateTimeFormat | null {
  return value === "LOCALE" || value === "ISO24H" ? value : null;
}

/**
 * The verification depth a select handed back, or `null` for anything the
 * server would not accept. Keeps the GraphQL enum's casing: the value goes
 * straight back into `updateVerificationSettings`.
 */
export function parseVerificationDepth(value: string): VerificationDepth | null {
  return value === "FULL" || value === "QUICK" ? value : null;
}

/**
 * `proxyConfigId` is included so the test dials the same egress live traffic
 * will use, including for a draft that has not been saved yet. Omitting it
 * tests the client directly.
 */
export function buildDownloadClientConnectionTestInput<TConfig>(
  id: string | null,
  clientType: string,
  config: TConfig,
  proxyConfigId: string | null = null,
) {
  const common = {
    clientType,
    config,
    ...(proxyConfigId ? { proxyConfigId } : {}),
  };
  return id ? { id, ...common } : common;
}
