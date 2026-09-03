export type OAuthAuthorizationRequest = {
  responseType: string;
  clientId: string;
  redirectUri: string;
  codeChallenge: string;
  codeChallengeMethod: string;
  scope: string | null;
  state: string | null;
};

type GraphQlErrorLike = {
  extensions?: Record<string, unknown> | null;
};

type OAuthErrorLike = {
  response?: { status?: number } | null;
  graphQLErrors?: readonly GraphQlErrorLike[];
};

export function oauthAuthorizationRequestFromSearch(search: string): OAuthAuthorizationRequest {
  const params = new URLSearchParams(search);
  return {
    responseType: params.get("response_type") ?? "",
    clientId: params.get("client_id") ?? "",
    redirectUri: params.get("redirect_uri") ?? "",
    codeChallenge: params.get("code_challenge") ?? "",
    codeChallengeMethod: params.get("code_challenge_method") ?? "",
    scope: params.get("scope"),
    state: params.get("state"),
  };
}

const PENDING_OAUTH_DECISION_KEY = "scryer.oauth.pending-authorization-decision";
/** A stored approval only survives the sign-in round trip, never a later visit. */
export const PENDING_OAUTH_DECISION_TTL_MS = 5 * 60 * 1000;

export type PendingOAuthDecision = {
  approved: true;
  fingerprint: string;
  expiresAt: number;
};

/**
 * Identifies the exact authorization request an approval was given for without persisting its
 * OAuth correlation values in recoverable form. A stored approval is replayed only when every
 * request parameter still matches.
 */
export async function oauthAuthorizationRequestFingerprint(
  request: OAuthAuthorizationRequest,
): Promise<string> {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) {
    throw new Error("Secure browser hashing is required to retain OAuth approval.");
  }
  const input = new TextEncoder().encode(JSON.stringify([
    request.responseType,
    request.clientId,
    request.redirectUri,
    request.scope ?? "",
    request.state ?? "",
    request.codeChallenge,
    request.codeChallengeMethod,
  ]));
  const digest = await subtle.digest("SHA-256", input.slice());
  return Array.from(
    new Uint8Array(digest),
    (byte) => byte.toString(16).padStart(2, "0"),
  ).join("");
}

export function isReplayablePendingOAuthDecision(
  stored: unknown,
  fingerprint: string,
  now: number,
): stored is PendingOAuthDecision {
  if (!stored || typeof stored !== "object") return false;
  const candidate = stored as Partial<PendingOAuthDecision>;
  return (
    candidate.approved === true
    && candidate.fingerprint === fingerprint
    && typeof candidate.expiresAt === "number"
    && candidate.expiresAt > now
  );
}

export async function pendingOAuthDecisionFor(
  request: OAuthAuthorizationRequest,
  now: number = Date.now(),
): Promise<PendingOAuthDecision> {
  return {
    approved: true,
    fingerprint: await oauthAuthorizationRequestFingerprint(request),
    expiresAt: now + PENDING_OAUTH_DECISION_TTL_MS,
  };
}

export async function storePendingOAuthDecision(
  request: OAuthAuthorizationRequest,
  now: number = Date.now(),
): Promise<void> {
  try {
    window.sessionStorage.setItem(
      PENDING_OAUTH_DECISION_KEY,
      JSON.stringify(await pendingOAuthDecisionFor(request, now)),
    );
  } catch {
    // A blocked session store just means the consent card is shown again after sign-in.
  }
}

export function clearPendingOAuthDecision(): void {
  try {
    window.sessionStorage.removeItem(PENDING_OAUTH_DECISION_KEY);
  } catch {
    // Nothing to clear when session storage is unavailable.
  }
}

/**
 * Consumes any stored approval. It is removed whether or not it matches, so a mismatched, expired,
 * or failing decision can never be replayed twice.
 */
export async function takePendingOAuthDecision(
  request: OAuthAuthorizationRequest,
  now: number = Date.now(),
): Promise<boolean> {
  let raw: string | null;
  try {
    raw = window.sessionStorage.getItem(PENDING_OAUTH_DECISION_KEY);
  } catch {
    return false;
  }
  clearPendingOAuthDecision();
  if (!raw) return false;
  try {
    return isReplayablePendingOAuthDecision(
      JSON.parse(raw),
      await oauthAuthorizationRequestFingerprint(request),
      now,
    );
  } catch {
    return false;
  }
}

export function isOAuthAuthenticationError(error: unknown): boolean {
  if (!error || typeof error !== "object") return false;
  const candidate = error as OAuthErrorLike;
  if (candidate.response?.status === 401) return true;
  return candidate.graphQLErrors?.some((graphQlError) => {
    const code = graphQlError.extensions?.code;
    return code === "UNAUTHORIZED" || code === "AUTHENTICATION_REQUIRED";
  }) ?? false;
}
