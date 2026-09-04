import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, Loader2, RefreshCw, ShieldCheck, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  authRuntimeStateQuery,
  meQuery,
  oauthAuthorizationClientQuery,
} from "@/lib/graphql/queries";
import { backendClient } from "@/lib/graphql/urql-client";
import { clearClientAuthSession, getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeBackendUrl, getRuntimeBasePath } from "@/lib/runtime-config";
import type { AuthRuntimeState } from "@/lib/types/settings";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  clearPendingOAuthDecision,
  isOAuthAuthenticationError,
  oauthAuthorizationRequestFromSearch,
  storePendingOAuthDecision,
  takePendingOAuthDecision,
  type OAuthAuthorizationRequest,
} from "@/lib/utils/oauth-authorization-request";

const OAUTH_PAGE_CLASS =
  "flex min-h-screen items-center justify-center bg-fixed p-4 text-[var(--scry-body)] [background-image:var(--scry-shell-bg)] sm:p-6";
const OAUTH_PANEL_CLASS =
  "grid w-full max-w-xl gap-5 rounded-[12px] border border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-7 shadow-[0_22px_70px_rgba(2,6,23,0.26)] max-sm:p-5";
const OAUTH_COMPACT_PANEL_CLASS =
  "grid w-full max-w-lg gap-5 rounded-[12px] border border-[var(--scry-border2)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-bg))] p-7 shadow-[0_22px_70px_rgba(2,6,23,0.26)] max-sm:p-5";
const OAUTH_ICON_CLASS =
  "flex h-11 w-11 items-center justify-center rounded-[10px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent-text)]";
const OAUTH_HEADING_CLASS =
  "font-[var(--font-space-grotesk)] text-2xl font-semibold tracking-normal text-[var(--scry-ink)]";
const OAUTH_URI_CLASS =
  "break-all rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2 font-[var(--font-code)] text-xs leading-5 text-[var(--scry-muted)]";
const OAUTH_ERROR_CLASS =
  "rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm leading-6 text-[var(--scry-danger-text)]";
const OAUTH_PRIMARY_BUTTON_CLASS =
  "h-10 rounded-[9px] bg-primary px-4 text-sm font-semibold text-primary-foreground shadow-none hover:bg-primary/90";
const OAUTH_SECONDARY_BUTTON_CLASS =
  "h-10 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-4 text-sm font-semibold text-[var(--scry-ink2)] shadow-none hover:bg-[var(--scry-hover)]";

type OAuthDecisionOutcome =
  | { kind: "redirect"; redirectUri: string }
  | { kind: "reauthenticate" }
  | { kind: "error"; message: string };

async function submitOAuthDecision(
  request: OAuthAuthorizationRequest,
  token: string | null,
  approved: boolean,
): Promise<OAuthDecisionOutcome> {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (token) headers.authorization = `Bearer ${token}`;
  const response = await fetch(getRuntimeBackendUrl("/oauth/authorize/decision"), {
    method: "POST",
    headers,
    body: JSON.stringify({
      approved,
      responseType: request.responseType,
      clientId: request.clientId,
      redirectUri: request.redirectUri,
      codeChallenge: request.codeChallenge,
      codeChallengeMethod: request.codeChallengeMethod,
      scope: request.scope,
      state: request.state,
    }),
  });
  const body = (await response.json().catch(() => null)) as
    | {
        redirectUri?: string;
        error?: string;
        errorDescription?: string;
        error_description?: string;
      }
    | null;
  if (response.ok && body?.redirectUri) {
    return { kind: "redirect", redirectUri: body.redirectUri };
  }
  if (body?.error === "reauthentication_required") return { kind: "reauthenticate" };
  return {
    kind: "error",
    message:
      body?.error_description
      ?? body?.errorDescription
      ?? "Unable to authorize this integration.",
  };
}

function loginUrl() {
  const basePath = getRuntimeBasePath();
  const loginPath = basePath === "/" ? "/login" : `${basePath}/login`;
  const path =
    basePath !== "/" && window.location.pathname.startsWith(basePath)
      ? window.location.pathname.slice(basePath.length) || "/"
      : window.location.pathname;
  const redirect = `${path}${window.location.search}${window.location.hash}`;
  const params = new URLSearchParams({ redirect });
  return `${loginPath}?${params.toString()}`;
}

export default function OAuthAuthorizePage() {
  const search = window.location.search;
  const request = useMemo(() => oauthAuthorizationRequestFromSearch(search), [search]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [actorValidationRetry, setActorValidationRetry] = useState(0);
  const [actorValidationRetryAvailable, setActorValidationRetryAvailable] = useState(false);
  const [clientName, setClientName] = useState<string | null>(null);
  const [authorizationScope, setAuthorizationScope] = useState<string | null>(null);
  const [actorUsername, setActorUsername] = useState<string | null>(null);
  const [requestValidationError, setRequestValidationError] = useState<string | null>(null);
  const [actorValidationError, setActorValidationError] = useState<string | null>(null);
  const [effectiveFormLoginEnabled, setEffectiveFormLoginEnabled] = useState<boolean | null>(null);
  const [completingAuthorization, setCompletingAuthorization] = useState(false);
  const token = getAuthToken();
  const authlessAuthorization = effectiveFormLoginEnabled === false;
  const awaitingAuthenticationCheck =
    !token && !authlessAuthorization && !requestValidationError;
  const approvalPreviewReady =
    !!clientName && !requestValidationError && (authlessAuthorization || !!actorUsername);
  const denialReady = !!clientName && !requestValidationError;
  const grantedScopes = authorizationScope?.split(" ") ?? [];
  // The backend narrows an unbacked jellyfin-link request to library-only, so the consent card
  // reports the scope that will actually be granted and says why it shrank.
  const jellyfinLinkScopeDropped =
    !authlessAuthorization
    && !!authorizationScope
    && (request.scope?.split(" ").includes("jellyfin-link") ?? false)
    && !grantedScopes.includes("jellyfin-link");

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        setActorValidationRetryAvailable(false);
        const runtimeResult = await backendClient
          .query<{ authRuntimeState?: AuthRuntimeState | null }>(authRuntimeStateQuery, {})
          .toPromise();
        if (cancelled) return;

        const runtimeState = runtimeResult.data?.authRuntimeState ?? null;
        const formLoginEnabled = runtimeState?.effectiveFormLoginEnabled !== false;
        setEffectiveFormLoginEnabled(formLoginEnabled);
        if (formLoginEnabled && !token) {
          window.location.assign(loginUrl());
          return;
        }

        const [clientResult, actorResult] = await Promise.all([
          backendClient
            .query<{
              oauthAuthorizationClient?: { clientId: string; displayName: string; scope: string } | null;
            }>(oauthAuthorizationClientQuery, {
              clientId: request.clientId,
              redirectUri: request.redirectUri,
              scope: request.scope,
            })
            .toPromise(),
          token
            ? backendClient.query<{ me?: { username?: string | null } | null }>(meQuery, {}).toPromise()
            : Promise.resolve(null),
        ]);
        if (cancelled) return;
        const authorizationClient = clientResult.data?.oauthAuthorizationClient ?? null;
        const username = actorResult?.data?.me?.username?.trim() || null;
        const actorVerificationFailed =
          runtimeState?.effectiveFormLoginEnabled !== false && (!username || actorResult?.error);
        if (authorizationClient?.displayName) {
          setClientName(authorizationClient.displayName);
          setAuthorizationScope(authorizationClient.scope);
          setActorUsername(username);
          setRequestValidationError(null);
        } else {
          setClientName(null);
          setAuthorizationScope(null);
          setActorUsername(null);
          setRequestValidationError(
            clientResult.error?.message ?? "This OAuth request has an invalid client or redirect URI.",
          );
        }
        if (actorVerificationFailed) {
          if (token && isOAuthAuthenticationError(actorResult?.error)) {
            clearClientAuthSession();
            setActorValidationError("Your Scryer session expired. Sign in again to continue.");
          } else {
            setActorValidationError(
              "Unable to verify the signed-in Scryer account for this OAuth request.",
            );
            setActorValidationRetryAvailable(!!token);
          }
        } else {
          setActorValidationError(null);
        }

        // A step-up sign-in already collected consent before the redirect to /login. Finish that
        // approval here rather than showing the same consent card a second time.
        const replayApproval =
          !!token
          && !actorVerificationFailed
          && !!authorizationClient?.displayName
          && await takePendingOAuthDecision(request);
        if (!replayApproval) return;
        setCompletingAuthorization(true);
        // A replay that cannot complete falls back to the ordinary consent card with an error,
        // never to another sign-in round trip.
        let outcome: OAuthDecisionOutcome;
        try {
          outcome = await submitOAuthDecision(request, token, true);
        } catch (replayError) {
          if (cancelled) return;
          setCompletingAuthorization(false);
          setError(
            replayError instanceof Error
              ? replayError.message
              : "Unable to authorize this integration.",
          );
          return;
        }
        if (cancelled) return;
        if (outcome.kind === "redirect") {
          window.location.assign(outcome.redirectUri);
          return;
        }
        setCompletingAuthorization(false);
        if (outcome.kind === "reauthenticate") {
          clearClientAuthSession();
          setError("Your session is no longer fresh. Sign in again to continue.");
          return;
        }
        setError(outcome.message);
      } catch {
        if (!cancelled) {
          setCompletingAuthorization(false);
          setEffectiveFormLoginEnabled(null);
          setClientName(null);
          setAuthorizationScope(null);
          setActorUsername(null);
          setRequestValidationError("Unable to validate this OAuth request.");
          setActorValidationError(null);
          setActorValidationRetryAvailable(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [actorValidationRetry, request, token]);

  const decide = async (approved: boolean) => {
    setBusy(true);
    setError(null);
    try {
      if ((approved && !approvalPreviewReady) || (!approved && !denialReady)) {
        setError(
          requestValidationError ??
            actorValidationError ??
            "Validating OAuth request. Please try again.",
        );
        return;
      }
      if (!authlessAuthorization && !token) {
        window.location.assign(loginUrl());
        return;
      }
      const outcome = await submitOAuthDecision(
        request,
        authlessAuthorization ? null : token,
        approved,
      );
      if (outcome.kind === "redirect") {
        window.location.assign(outcome.redirectUri);
        return;
      }
      if (outcome.kind === "reauthenticate") {
        clearClientAuthSession();
        // Carry the approval across the sign-in so the user is not asked to consent twice.
        // A denial is never carried: it is not replayed on the user's behalf.
        if (approved) {
          await storePendingOAuthDecision(request);
          window.location.assign(loginUrl());
          return;
        }
        setError("Your session is no longer fresh. Sign in again to continue.");
        return;
      }
      setError(outcome.message);
    } catch (err) {
      clearPendingOAuthDecision();
      setError(err instanceof Error ? err.message : "Unable to authorize this integration.");
    } finally {
      setBusy(false);
    }
  };

  if (completingAuthorization) {
    return (
      <main className={OAUTH_PAGE_CLASS}>
        <div className={OAUTH_COMPACT_PANEL_CLASS}>
          <div
            id={selectorId("oauth-authorize-completing")}
            className="flex items-center justify-center gap-2 py-3 text-sm text-[var(--scry-muted)]"
          >
            <Loader2 className="h-5 w-5 animate-spin" aria-hidden="true" />
            Completing authorization…
          </div>
        </div>
      </main>
    );
  }

  if (awaitingAuthenticationCheck) {
    return (
      <main className={OAUTH_PAGE_CLASS}>
        <div className={OAUTH_COMPACT_PANEL_CLASS}>
          <div className="flex items-center justify-center py-3">
            <Loader2 className="h-5 w-5 animate-spin text-[var(--scry-muted)]" aria-label="Preparing authorization" />
          </div>
        </div>
      </main>
    );
  }

  return (
    <main className={OAUTH_PAGE_CLASS}>
      <div className={OAUTH_PANEL_CLASS}>
        <div className="flex items-start gap-4">
          <span className={OAUTH_ICON_CLASS} aria-hidden="true">
            <ShieldCheck className="h-5 w-5" />
          </span>
          <div className="min-w-0 space-y-2">
            <h1 id={selectorId("oauth-authorize-heading")} className={OAUTH_HEADING_CLASS}>
              Authorize {clientName ?? "integration"}
            </h1>
            <p className={OAUTH_URI_CLASS}>{request.redirectUri}</p>
          </div>
        </div>
        <div className="grid gap-3 rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-4 text-sm text-[var(--scry-ink2)]">
          <h2 className="font-medium text-[var(--scry-ink)]">Scopes</h2>
          <ul className="grid gap-2" aria-label="Granted scopes">
            <li className="flex items-center gap-2">
              <CheckCircle2 className="h-4 w-4 text-emerald-500" aria-hidden="true" />
              Library access
            </li>
            {!authlessAuthorization && grantedScopes.includes("jellyfin-link") ? (
              <li className="flex items-center gap-2">
                <CheckCircle2 className="h-4 w-4 text-emerald-500" aria-hidden="true" />
                Jellyfin account linking
              </li>
            ) : null}
          </ul>
          {jellyfinLinkScopeDropped ? (
            <p
              id={selectorId("oauth-authorize-link-scope-dropped")}
              className="text-xs text-[var(--scry-muted)]"
            >
              Account linking not granted: no eligible Jellyfin connection.
            </p>
          ) : null}
        </div>
        {error ?? requestValidationError ?? actorValidationError ? (
          <p className={OAUTH_ERROR_CLASS}>
            {error ?? requestValidationError ?? actorValidationError}
          </p>
        ) : null}
        <div className="flex flex-wrap gap-2">
          <Button
            id={selectorId("oauth-authorize-approve")}
            disabled={busy || !approvalPreviewReady}
            className={OAUTH_PRIMARY_BUTTON_CLASS}
            onClick={() => decide(true)}
          >
            {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
            {authlessAuthorization ? "Authorize as Anonymous" : "Authorize"}
          </Button>
          {actorValidationRetryAvailable ? (
            <Button
              id={selectorId("oauth-authorize-retry-validation")}
              variant="outline"
              disabled={busy}
              className={OAUTH_SECONDARY_BUTTON_CLASS}
              onClick={() => setActorValidationRetry((attempt) => attempt + 1)}
            >
              <RefreshCw className="h-4 w-4" aria-hidden="true" />
              Retry validation
            </Button>
          ) : null}
          <Button
            id={selectorId("oauth-authorize-deny")}
            variant="outline"
            disabled={busy || !denialReady}
            className={OAUTH_SECONDARY_BUTTON_CLASS}
            onClick={() => decide(false)}
          >
            <X className="h-4 w-4" aria-hidden="true" />
            Deny
          </Button>
        </div>
      </div>
    </main>
  );
}
