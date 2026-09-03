import {
  cacheExchange,
  Client,
  fetchExchange,
  subscriptionExchange,
} from "@urql/core";
import { getAuthlessWebClientProof } from "@/lib/authless-web-client";
import {
  clearClientAuthSession,
  getAuthToken,
  getMfaEnrollmentToken,
  getPasswordChangeRequiredToken,
} from "@/lib/hooks/use-auth";
import { getRuntimeBasePath, getRuntimeGraphqlUrl } from "@/lib/runtime-config";
import { wsClient } from "@/lib/graphql/ws-client";

// ---------------------------------------------------------------------------
// Shared language ref — updated by the Provider when uiLanguage changes
// ---------------------------------------------------------------------------

let currentLanguage = "eng";

export function setGraphqlLanguage(lang: string) {
  currentLanguage = lang;
}

/**
 * Read the language the Provider last published. Queries that take an explicit
 * `language` input (metadata lookups) use this instead of threading uiLanguage
 * through every component that can open a metadata-backed picker.
 */
export function getGraphqlLanguage(): string {
  return currentLanguage;
}

// ---------------------------------------------------------------------------
// Backend restart detection — when the backend returns HTML (upgrade splash)
// instead of JSON, trigger a global callback so the shell can show the splash
// overlay immediately, regardless of which component made the request.
// ---------------------------------------------------------------------------

let onBackendRestarting: (() => void) | null = null;

export const MFA_STEP_UP_REQUIRED_EVENT = "scryer:mfa-step-up-required";
const MFA_STEP_UP_REQUIRED_STATUS = 460;
const AUTHLESS_PROOF_REQUIRED_ERROR =
  "Scryer web client proof is required for unauthenticated access";

export function setOnBackendRestarting(cb: (() => void) | null) {
  onBackendRestarting = cb;
}

export function notifyBackendRestarting() {
  onBackendRestarting?.();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function dispatchMfaStepUpRequired() {
  if (typeof window === "undefined") {
    return;
  }

  window.dispatchEvent(new CustomEvent(MFA_STEP_UP_REQUIRED_EVENT));
}

function getLoginRedirectTarget() {
  if (typeof window === "undefined") {
    return null;
  }

  const basePath = getRuntimeBasePath();
  const pathname = window.location.pathname;

  if (basePath !== "/" && pathname.startsWith(basePath)) {
    return `${pathname.slice(basePath.length) || "/"}${window.location.search}${window.location.hash}`;
  }

  return `${pathname}${window.location.search}${window.location.hash}`;
}

function redirectToLogin() {
  if (typeof window === "undefined") {
    return;
  }

  const basePath = getRuntimeBasePath();
  const redirectTarget = getLoginRedirectTarget();
  const currentAppPath = redirectTarget ?? "/";
  clearClientAuthSession();

  if (currentAppPath === "/login" || currentAppPath.startsWith("/login?")) {
    return;
  }

  const loginPath = basePath === "/" ? "/login" : `${basePath}/login`;
  const params = new URLSearchParams();
  if (redirectTarget && redirectTarget.startsWith("/") && !redirectTarget.startsWith("//")) {
    params.set("redirect", redirectTarget);
  }

  const destination = params.size > 0 ? `${loginPath}?${params.toString()}` : loginPath;
  window.location.replace(destination);
}

function getHealthUrl() {
  const basePath = getRuntimeBasePath();
  return basePath === "/" ? "/health" : `${basePath}/health`;
}

async function backendHealthLooksOk(): Promise<boolean> {
  try {
    const response = await fetch(getHealthUrl(), {
      method: "GET",
      cache: "no-store",
      headers: {
        accept: "application/json",
      },
    });
    if (!response.ok) {
      return false;
    }
    const body = await response.json().catch(() => null);
    return isRecord(body) && body.status === "ok";
  } catch {
    return false;
  }
}

function inputPath(input: RequestInfo | URL): string | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const raw = typeof input === "string" || input instanceof URL ? input.toString() : input.url;
    return new URL(raw, window.location.origin).pathname;
  } catch {
    return null;
  }
}

async function withAuthlessProof(input: RequestInfo | URL, init?: RequestInit) {
  if (
    typeof window === "undefined" ||
    headersHaveAuthorization(init?.headers) ||
    getAuthToken()
  ) {
    return init;
  }
  const graphqlPath = new URL(getRuntimeGraphqlUrl(), window.location.origin).pathname;
  if (inputPath(input) !== graphqlPath) {
    return init;
  }
  const proof = await getAuthlessWebClientProof();
  if (!proof) {
    return init;
  }
  const headers = new Headers(init?.headers);
  headers.set("x-scryer-web-client", proof);
  return { ...init, credentials: "include" as RequestCredentials, headers };
}

function headersHaveAuthorization(headersInit?: HeadersInit): boolean {
  if (!headersInit) {
    return false;
  }
  return new Headers(headersInit).has("authorization");
}

function withoutAuthorization(init?: RequestInit): RequestInit | undefined {
  if (!init?.headers) {
    return init;
  }
  const headers = new Headers(init.headers);
  headers.delete("authorization");
  return { ...init, headers };
}

async function responseRequiresAuthlessProof(response: Response): Promise<boolean> {
  if (response.status !== 403) {
    return false;
  }
  try {
    const body = (await response.clone().json()) as unknown;
    return isRecord(body) && body.error === AUTHLESS_PROOF_REQUIRED_ERROR;
  } catch {
    return false;
  }
}

export const scryerFetch: typeof fetch = async (input, init) => {
  let response = await fetch(input, await withAuthlessProof(input, init));
  const hadClientAuth =
    headersHaveAuthorization(init?.headers) || getAuthToken() !== null;
  if (
    hadClientAuth &&
    (await responseRequiresAuthlessProof(response))
  ) {
    clearClientAuthSession();
    response = await fetch(
      input,
      await withAuthlessProof(input, withoutAuthorization(init)),
    );
  }
  if (response.status === 401) {
    redirectToLogin();
    throw new TypeError("Authentication required");
  }
  if (response.status === MFA_STEP_UP_REQUIRED_STATUS) {
    dispatchMfaStepUpRequired();
  }

  const ct = response.headers.get("content-type") ?? "";
  if (ct.includes("text/html")) {
    const backendHealthy = await backendHealthLooksOk();
    if (!backendHealthy) {
      onBackendRestarting?.();
      throw new TypeError("Service is restarting");
    }
    throw new TypeError("Unexpected HTML response from backend");
  }

  return response;
};

function errorHasName(error: unknown, name: string): boolean {
  return (
    error != null &&
    typeof error === "object" &&
    "name" in error &&
    (error as { name?: unknown }).name === name
  );
}

export function makeAbortableFetch(signal: AbortSignal): typeof fetch {
  return (input, init) => scryerFetch(input, { ...init, signal });
}

export function isAbortError(error: unknown): boolean {
  if (errorHasName(error, "AbortError")) {
    return true;
  }
  if (
    error != null &&
    typeof error === "object" &&
    "networkError" in error
  ) {
    return errorHasName(
      (error as { networkError?: unknown }).networkError,
      "AbortError",
    );
  }
  return false;
}

// ---------------------------------------------------------------------------
// Backend client — connects to the Rust GraphQL server at /graphql
// ---------------------------------------------------------------------------

export const backendClient = new Client({
  url: getRuntimeGraphqlUrl(),
  preferGetMethod: false,
  requestPolicy: "network-only",
  fetch: scryerFetch,
  // Keep the lightweight exchange available, but default every request to the network.
  exchanges: [
    cacheExchange,
    subscriptionExchange({
      forwardSubscription(request) {
        const input = { ...request, query: request.query || "" };
        return {
          subscribe(sink) {
            const unsubscribe = wsClient.subscribe(input, sink);
            return { unsubscribe };
          },
        };
      },
    }),
    fetchExchange,
  ],
  fetchOptions: () => {
    const headers: Record<string, string> = {
      "x-scryer-language": currentLanguage,
    };
    const token = getAuthToken();
    if (token) {
      headers["authorization"] = `Bearer ${token}`;
    }
    return { headers };
  },
});

export const mfaEnrollmentClient = new Client({
  url: getRuntimeGraphqlUrl(),
  preferGetMethod: false,
  requestPolicy: "network-only",
  fetch: scryerFetch,
  exchanges: [fetchExchange],
  fetchOptions: () => {
    const headers: Record<string, string> = {
      "x-scryer-language": currentLanguage,
    };
    const token = getMfaEnrollmentToken();
    if (token) {
      headers["authorization"] = `Bearer ${token}`;
    }
    return { headers };
  },
});

export const passwordChangeRequiredClient = new Client({
  url: getRuntimeGraphqlUrl(),
  preferGetMethod: false,
  requestPolicy: "network-only",
  fetch: scryerFetch,
  exchanges: [fetchExchange],
  fetchOptions: () => {
    const headers: Record<string, string> = {
      "x-scryer-language": currentLanguage,
    };
    const token = getPasswordChangeRequiredToken();
    if (token) {
      headers.authorization = `Bearer ${token}`;
    }
    return { headers };
  },
});
