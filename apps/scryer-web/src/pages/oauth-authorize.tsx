import { useEffect, useMemo, useState } from "react";
import { Loader2, LogIn, ShieldCheck, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  authRuntimeStateQuery,
  oauthAuthorizationClientQuery,
} from "@/lib/graphql/queries";
import { backendClient } from "@/lib/graphql/urql-client";
import { clearClientAuthSession, getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeBackendUrl, getRuntimeBasePath } from "@/lib/runtime-config";
import type { AuthRuntimeState } from "@/lib/types/settings";
import { selectorId } from "@/lib/utils/dom-ids";

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
const OAUTH_MUTED_TEXT_CLASS = "text-sm leading-6 text-[var(--scry-muted)]";
const OAUTH_URI_CLASS =
  "break-all rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2 font-[var(--font-code)] text-xs leading-5 text-[var(--scry-muted)]";
const OAUTH_ERROR_CLASS =
  "rounded-[9px] border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm leading-6 text-[var(--scry-danger-text)]";
const OAUTH_PRIMARY_BUTTON_CLASS =
  "h-10 rounded-[9px] bg-primary px-4 text-sm font-semibold text-primary-foreground shadow-none hover:bg-primary/90";
const OAUTH_SECONDARY_BUTTON_CLASS =
  "h-10 rounded-[9px] border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-4 text-sm font-semibold text-[var(--scry-ink2)] shadow-none hover:bg-[var(--scry-hover)]";

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
  const params = useMemo(() => new URLSearchParams(window.location.search), []);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reauthenticationRequired, setReauthenticationRequired] = useState(false);
  const [clientName, setClientName] = useState<string | null>(null);
  const [clientValidationError, setClientValidationError] = useState<string | null>(null);
  const [effectiveFormLoginEnabled, setEffectiveFormLoginEnabled] = useState<boolean | null>(null);
  const clientId = params.get("client_id") ?? "";
  const redirectUri = params.get("redirect_uri") ?? "";
  const token = getAuthToken();
  const authlessAuthorization = effectiveFormLoginEnabled === false;

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const [runtimeResult, clientResult] = await Promise.all([
          backendClient
            .query<{ authRuntimeState?: AuthRuntimeState | null }>(authRuntimeStateQuery, {})
            .toPromise(),
          backendClient
            .query<{
              oauthAuthorizationClient?: { clientId: string; displayName: string } | null;
            }>(oauthAuthorizationClientQuery, { clientId, redirectUri })
            .toPromise(),
        ]);
        if (!cancelled) {
          const runtimeState = runtimeResult.data?.authRuntimeState ?? null;
          setEffectiveFormLoginEnabled(
            typeof runtimeState?.effectiveFormLoginEnabled === "boolean"
              ? runtimeState.effectiveFormLoginEnabled
              : null,
          );
          const authorizationClient = clientResult.data?.oauthAuthorizationClient ?? null;
          if (authorizationClient?.displayName) {
            setClientName(authorizationClient.displayName);
            setClientValidationError(null);
          } else {
            setClientName(null);
            setClientValidationError(
              clientResult.error?.message ?? "This OAuth request has an invalid client or redirect URI.",
            );
          }
        }
      } catch {
        if (!cancelled) {
          setEffectiveFormLoginEnabled(null);
          setClientName(null);
          setClientValidationError("Unable to validate this OAuth request.");
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [clientId, redirectUri]);

  const decide = async (approved: boolean) => {
    setBusy(true);
    setError(null);
    try {
      if (!clientName) {
        setError(clientValidationError ?? "Validating OAuth request. Please try again.");
        return;
      }
      if (!authlessAuthorization && !token) {
        window.location.assign(loginUrl());
        return;
      }
      const headers: Record<string, string> = {
        "content-type": "application/json",
      };
      if (!authlessAuthorization && token) {
        headers.authorization = `Bearer ${token}`;
      }
      const response = await fetch(getRuntimeBackendUrl("/oauth/authorize/decision"), {
        method: "POST",
        headers,
        body: JSON.stringify({
          approved,
          responseType: params.get("response_type") ?? "",
          clientId,
          redirectUri,
          codeChallenge: params.get("code_challenge") ?? "",
          codeChallengeMethod: params.get("code_challenge_method") ?? "",
          scope: params.get("scope"),
          state: params.get("state"),
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
      if (!response.ok || !body?.redirectUri) {
        if (body?.error === "reauthentication_required") {
          clearClientAuthSession();
          setReauthenticationRequired(true);
          setError("Your session is no longer fresh. Sign in again to continue.");
          return;
        }
        setError(
          body?.error_description ??
            body?.errorDescription ??
            "Unable to authorize this integration.",
        );
        return;
      }
      window.location.assign(body.redirectUri);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Unable to authorize this integration.");
    } finally {
      setBusy(false);
    }
  };

  if (!authlessAuthorization && !token && clientName) {
    return (
      <main className={OAUTH_PAGE_CLASS}>
        <div className={OAUTH_COMPACT_PANEL_CLASS}>
          <div className="flex items-start gap-4">
            <span className={OAUTH_ICON_CLASS} aria-hidden="true">
              <LogIn className="h-5 w-5" />
            </span>
            <div className="min-w-0 space-y-1">
              <h1 id={selectorId("oauth-authorize-heading")} className={OAUTH_HEADING_CLASS}>
                Authorize {clientName ?? "integration"}
              </h1>
              <p className={OAUTH_MUTED_TEXT_CLASS}>
                Sign in to continue OAuth authorization.
              </p>
            </div>
          </div>
          <Button
            id={selectorId("oauth-authorize-sign-in")}
            className={OAUTH_PRIMARY_BUTTON_CLASS}
            onClick={() => window.location.assign(loginUrl())}
          >
            <LogIn className="h-4 w-4" aria-hidden="true" />
            Sign in
          </Button>
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
            <p className={OAUTH_URI_CLASS}>{redirectUri}</p>
          </div>
        </div>
        <div className="grid gap-2 rounded-[9px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-4 text-sm leading-6 text-[var(--scry-ink2)]">
          <p>
            Can access Scryer as {authlessAuthorization ? "Anonymous" : "you"}, limited to library
            permissions.
          </p>
          <p className="text-[var(--scry-muted)]">
            Cannot manage users, settings, backups, security, or app configuration.
          </p>
        </div>
        {error ?? clientValidationError ? (
          <p className={OAUTH_ERROR_CLASS}>{error ?? clientValidationError}</p>
        ) : null}
        <div className="flex flex-wrap gap-2">
          {reauthenticationRequired ? (
            <Button
              id={selectorId("oauth-authorize-sign-in-again")}
              className={OAUTH_PRIMARY_BUTTON_CLASS}
              onClick={() => window.location.assign(loginUrl())}
            >
              <LogIn className="h-4 w-4" aria-hidden="true" />
              Sign in again
            </Button>
          ) : (
            <Button
              id={selectorId("oauth-authorize-approve")}
              disabled={busy || !clientName}
              className={OAUTH_PRIMARY_BUTTON_CLASS}
              onClick={() => decide(true)}
            >
              {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {authlessAuthorization ? "Authorize as Anonymous" : "Authorize"}
            </Button>
          )}
          <Button
            id={selectorId("oauth-authorize-deny")}
            variant="outline"
            disabled={busy || !clientName}
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
