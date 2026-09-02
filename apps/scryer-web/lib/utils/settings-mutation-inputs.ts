import type {
  ProxyDraft,
  UiDateTimeFormat,
  VerificationDepth,
} from "../types/index.ts";
// Imported from the module rather than the barrel: these are runtime values,
// and the barrel's extensionless re-exports only resolve for erased types.
import {
  supportsProxyCredentials,
  supportsProxyRemoteDns,
} from "../types/indexers.ts";

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
 */
function buildProxyCredentialInput(
  draft: ProxyDraft,
  { allowClear }: { allowClear: boolean },
): { username?: string | null; password?: string | null } {
  if (!supportsProxyCredentials(draft.providerType)) {
    return {};
  }
  if (allowClear && draft.clearCredentials) {
    return { username: null, password: null };
  }
  const username = draft.username.trim();
  const password = draft.password.trim();
  return {
    ...(username ? { username } : {}),
    ...(password ? { password } : {}),
  };
}

export function buildCreateProxyInput(draft: ProxyDraft) {
  return {
    providerType: draft.providerType,
    ...buildProxyCommonInput(draft),
    ...buildProxyRemoteDnsInput(draft),
    // Nothing is stored yet, so there is nothing to clear.
    ...buildProxyCredentialInput(draft, { allowClear: false }),
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
  };
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

export function buildDownloadClientConnectionTestInput<TConfig>(
  id: string | null,
  clientType: string,
  config: TConfig,
) {
  const common = { clientType, config };
  return id ? { id, ...common } : common;
}
