export const LIST_ACCOUNT_LINK_TYPE = "scryer:list-account-link";
export const LIST_ACCOUNT_LINK_STORAGE_KEY = "scryer:list-account-link:pending";
export const LIST_ACCOUNT_RELAY_ORIGIN = "https://smg.scryer.media";
const PROVIDERS = new Set(["plex", "tmdb", "trakt", "anilist", "mal", "simkl", "mdblist"]);
const PAYLOAD_KEYS = new Set(["type", "provider", "state", "exchange_code", "code", "iss", "error"]);

export type PendingListAccountLink = {
  sessionId: string;
  state: string;
  provider: string;
  authorizationOrigin: string;
  expiresAt: string;
};

export type ListAccountLinkPayload = {
  type: typeof LIST_ACCOUNT_LINK_TYPE;
  provider: string;
  state: string;
  exchange_code?: string;
  code?: string;
  iss?: string;
  error?: string;
};

function boundedString(value: unknown, max = 8192): value is string {
  return typeof value === "string" && value.length > 0 && value.length <= max;
}

export function parseListAccountPayload(value: unknown): ListAccountLinkPayload | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const data = value as Record<string, unknown>;
  if (Object.keys(data).some((key) => !PAYLOAD_KEYS.has(key))) return null;
  if (data.type !== LIST_ACCOUNT_LINK_TYPE || !boundedString(data.provider, 32) || !PROVIDERS.has(data.provider)) return null;
  if (!boundedString(data.state)) return null;
  if (data.iss !== undefined && !boundedString(data.iss, 256)) return null;
  if (data.error !== undefined && !boundedString(data.error, 256)) return null;
  if (data.exchange_code !== undefined && !boundedString(data.exchange_code)) return null;
  if (data.code !== undefined && !boundedString(data.code)) return null;
  if (!data.error && !data.exchange_code && !data.code) return null;
  if (data.error && (data.exchange_code || data.code)) return null;
  if (data.exchange_code && data.code) return null;
  if (data.provider === "simkl" && data.iss !== "https://simkl.com") return null;
  return data as ListAccountLinkPayload;
}

export function parsePendingListAccountLink(raw: string | null, origin: string, now = Date.now()): PendingListAccountLink | null {
  try {
    const value = JSON.parse(raw ?? "null") as Partial<PendingListAccountLink> | null;
    if (!value || !boundedString(value.sessionId, 256) || !boundedString(value.state) || !boundedString(value.provider, 32) || !PROVIDERS.has(value.provider)) return null;
    if (value.authorizationOrigin !== origin && value.authorizationOrigin !== LIST_ACCOUNT_RELAY_ORIGIN) return null;
    if (!boundedString(value.expiresAt, 64) || !Number.isFinite(Date.parse(value.expiresAt)) || Date.parse(value.expiresAt) <= now) return null;
    return value as PendingListAccountLink;
  } catch {
    return null;
  }
}

export function validateListAccountMessage(
  event: { origin: string; source: unknown; data: unknown },
  pending: PendingListAccountLink,
  popup: unknown,
  instanceOrigin: string,
  consumed: boolean,
  now = Date.now(),
): ListAccountLinkPayload | null {
  if (consumed || !popup || event.source !== popup || Date.parse(pending.expiresAt) <= now) return null;
  if (event.origin !== pending.authorizationOrigin && event.origin !== instanceOrigin) return null;
  const payload = parseListAccountPayload(event.data);
  if (!payload || payload.provider !== pending.provider || payload.state !== pending.state) return null;
  return payload;
}

/** Erase callback material synchronously, before parsing or performing a request. */
export function consumeListAccountReturn(
  location: Pick<Location, "pathname" | "search" | "hash">,
  history: Pick<History, "replaceState">,
  pending: PendingListAccountLink | null,
): ListAccountLinkPayload | null {
  const hash = location.hash;
  const query = location.search;
  history.replaceState(null, "", location.pathname);
  try {
    if (hash.length > 16000 || query.length > 16000) return null;
    if (location.pathname.endsWith("/lists/oauth/return")) {
      return parseListAccountPayload(JSON.parse(decodeURIComponent(hash.slice(1))));
    }
    if (!location.pathname.endsWith("/lists/oauth/callback") || !pending) return null;
    const params = new URLSearchParams(query);
    if ([...params.keys()].some((key) => !["state", "code", "iss", "error"].includes(key))) return null;
    if (["state", "code", "iss", "error"].some((key) => params.getAll(key).length > 1)) return null;
    return parseListAccountPayload({
      type: LIST_ACCOUNT_LINK_TYPE,
      provider: pending.provider,
      state: params.get("state"),
      ...(params.get("code") ? { code: params.get("code") } : {}),
      ...(params.get("iss") ? { iss: params.get("iss") } : {}),
      ...(params.get("error") ? { error: params.get("error") } : {}),
    });
  } catch {
    return null;
  }
}

export function listAccountReturnMatches(payload: ListAccountLinkPayload | null, pending: PendingListAccountLink | null): boolean {
  return !!payload && !!pending && payload.provider === pending.provider && payload.state === pending.state;
}

export function captureListAccountReturn(
  location: Pick<Location, "pathname" | "search" | "hash" | "origin">,
  history: Pick<History, "replaceState">,
  readPending: () => string | null,
) {
  const captured = { pathname: location.pathname, search: location.search, hash: location.hash };
  history.replaceState(null, "", captured.pathname);
  let pending: PendingListAccountLink | null = null;
  try { pending = parsePendingListAccountLink(readPending(), location.origin); }
  catch { /* Storage can be unavailable; callback material has already been erased. */ }
  return { pending, payload: consumeListAccountReturn(captured, history, pending) };
}

export function listAccountCompletionInput(pending: PendingListAccountLink, payload: ListAccountLinkPayload) {
  return {
    sessionId: pending.sessionId,
    state: payload.state,
    provider: payload.provider,
    code: payload.exchange_code ?? payload.code ?? "",
    issuer: payload.iss ?? null,
  };
}

export async function finishCurrentListAccountLink(
  isCurrent: () => boolean,
  refresh: (isCurrent: () => boolean) => Promise<void>,
  finish: () => void,
  fail: (reason: unknown) => void,
): Promise<void> {
  if (!isCurrent()) return;
  try {
    await refresh(isCurrent);
    if (isCurrent()) finish();
  } catch (reason) {
    if (isCurrent()) fail(reason);
  }
}
