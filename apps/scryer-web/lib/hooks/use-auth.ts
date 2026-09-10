import { useCallback, useEffect, useRef, useState } from "react";
import { backendClient } from "@/lib/graphql/urql-client";
import { decodeJwtPayload, isTokenExpired } from "@/lib/utils/jwt";
import { authRuntimeStateQuery, meQuery } from "@/lib/graphql/queries";
import { loginMutation } from "@/lib/graphql/mutations";
import type { AuthRuntimeState } from "@/lib/types/settings";
import type { UserAccountKind } from "@/lib/types/users";
import {
  PERSISTENT_STORAGE_KEY,
  SESSION_STORAGE_KEY,
  storeAuthToken,
} from "@/lib/utils/auth-session-persistence";
import {
  normalizeJwtPermissionClaims,
  type AppPermission,
  type LibraryPermissionGrant,
} from "@/lib/utils/permissions";

export const AUTH_SESSION_CHANGED_EVENT = "scryer:auth-session-changed";

export type AuthUser = {
  id: string;
  username: string;
  hasPassword?: boolean;
  hasMfa?: boolean;
  hasPasskey?: boolean;
  accountKind?: UserAccountKind;
  appPermissions: AppPermission[];
  libraryPermissions: LibraryPermissionGrant[];
};
type AuthLoginOptions = { persistSession?: boolean; totpCode?: string | null };
type AuthLoginResult = {
  token: string;
  user: AuthUser | null;
  mfaEnrollmentRequired: boolean;
  passwordChangeRequired: boolean;
  persistSession: boolean;
};

// Module-level token ref so getAuthToken() can be called outside React
let currentToken: string | null = null;

export function getAuthToken(): string | null {
  if (currentToken) {
    if (userFromToken(currentToken)) {
      return currentToken;
    }
    if (isRestrictedAuthToken(currentToken)) {
      return null;
    }
    currentToken = null;
  }

  if (typeof window === "undefined") {
    return null;
  }

  const stored =
    window.sessionStorage.getItem(SESSION_STORAGE_KEY) ??
    window.localStorage.getItem(PERSISTENT_STORAGE_KEY);
  if (!stored) {
    return null;
  }

  if (isRestrictedAuthToken(stored)) {
    currentToken = stored;
    return null;
  }

  if (!userFromToken(stored)) {
    clearPersistedAuthToken();
    return null;
  }

  currentToken = stored;
  return stored;
}

export function getMfaEnrollmentToken(): string | null {
  if (currentToken && isMfaEnrollmentToken(currentToken)) {
    return currentToken;
  }

  if (typeof window === "undefined") {
    return null;
  }

  const stored =
    window.sessionStorage.getItem(SESSION_STORAGE_KEY) ??
    window.localStorage.getItem(PERSISTENT_STORAGE_KEY);
  if (!stored || !isMfaEnrollmentToken(stored)) {
    return null;
  }

  currentToken = stored;
  return stored;
}

/** Returns the short-lived, session-only token for password replacement. */
export function getPasswordChangeRequiredToken(): string | null {
  if (currentToken && isPasswordChangeRequiredToken(currentToken)) {
    return currentToken;
  }

  if (typeof window === "undefined") {
    return null;
  }

  const stored = window.sessionStorage.getItem(SESSION_STORAGE_KEY);
  if (!stored || !isPasswordChangeRequiredToken(stored)) {
    return null;
  }

  currentToken = stored;
  return stored;
}

export function clearClientAuthSession() {
  clearPersistedAuthToken();
}

function dispatchAuthSessionChanged() {
  if (typeof window === "undefined") {
    return;
  }
  window.dispatchEvent(new CustomEvent(AUTH_SESSION_CHANGED_EVENT));
}

function persistAuthToken(token: string, persistSession: boolean) {
  storeAuthToken(token, persistSession);
  currentToken = token;
  dispatchAuthSessionChanged();
}

function clearPersistedAuthToken() {
  const hadCurrentToken = currentToken !== null;
  const hadPersistedToken =
    typeof window !== "undefined" &&
    (sessionStorage.getItem(SESSION_STORAGE_KEY) !== null ||
      localStorage.getItem(PERSISTENT_STORAGE_KEY) !== null);

  if (typeof window !== "undefined") {
    sessionStorage.removeItem(SESSION_STORAGE_KEY);
    localStorage.removeItem(PERSISTENT_STORAGE_KEY);
  }
  currentToken = null;
  if (hadCurrentToken || hadPersistedToken) {
    dispatchAuthSessionChanged();
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function isRateLimitedError(error: unknown): boolean {
  if (!isRecord(error) || !Array.isArray(error.graphQLErrors)) {
    return false;
  }

  return error.graphQLErrors.some((entry) => {
    if (!isRecord(entry)) {
      return false;
    }

    const extensions = isRecord(entry.extensions) ? entry.extensions : null;
    if (extensions?.code === "RATE_LIMITED") {
      return true;
    }

    return (
      typeof entry.message === "string" &&
      entry.message.trim().toLowerCase() === "api rate limit exceeded"
    );
  });
}

async function waitForRateLimitWindow(attempt: number) {
  const delayMs = [250, 500, 1_000][Math.min(attempt, 2)];
  await new Promise((resolve) => window.setTimeout(resolve, delayMs));
}

type AuthBootstrapSnapshot = {
  token: string | null;
  user: AuthUser | null;
  passwordChangeRequired: boolean;
  effectiveFormLoginEnabled: boolean | null;
  passkeyEnabled: boolean;
  defaultPersistSession: boolean;
  mfaRequirePasswordLogin: boolean;
  mfaRequireConfigStepUp: boolean | null;
  mfaRequireJellyfinLogin: boolean;
};

let authBootstrapSnapshot: AuthBootstrapSnapshot | null = null;
let authBootstrapPromise: Promise<AuthBootstrapSnapshot> | null = null;

async function queryWithRateLimitRetry<TData>(
  query: string,
): Promise<TData | null> {
  for (let attempt = 0; attempt < 3; attempt += 1) {
    const { data, error } = await backendClient.query(query, {}).toPromise();
    if (!error) {
      return (data as TData | null) ?? null;
    }
    if (!isRateLimitedError(error) || attempt === 2) {
      throw error;
    }
    await waitForRateLimitWindow(attempt);
  }

  return null;
}

function normalizeAuthUser(user: AuthUser | null | undefined): AuthUser | null {
  if (!user) {
    return null;
  }

  return {
    ...user,
    ...normalizeJwtPermissionClaims(
      user.appPermissions,
      user.libraryPermissions,
    ),
  };
}

async function loadUserFromBypass(options?: { clearToken?: boolean }) {
  if (options?.clearToken) {
    clearPersistedAuthToken();
  }

  try {
    const data = await queryWithRateLimitRetry<{ me?: AuthUser | null }>(
      meQuery,
    );
    return normalizeAuthUser(data?.me);
  } catch {
    return null;
  }
}

function rememberAuthBootstrapSession(token: string | null, user: AuthUser | null) {
  authBootstrapPromise = null;
  authBootstrapSnapshot = {
    token,
    user,
    passwordChangeRequired: Boolean(
      token && isPasswordChangeRequiredToken(token),
    ),
    effectiveFormLoginEnabled:
      authBootstrapSnapshot?.effectiveFormLoginEnabled ?? null,
    passkeyEnabled: authBootstrapSnapshot?.passkeyEnabled ?? false,
    defaultPersistSession: authBootstrapSnapshot?.defaultPersistSession ?? false,
    mfaRequirePasswordLogin:
      authBootstrapSnapshot?.mfaRequirePasswordLogin ?? false,
    mfaRequireConfigStepUp:
      authBootstrapSnapshot?.mfaRequireConfigStepUp ?? null,
    mfaRequireJellyfinLogin:
      authBootstrapSnapshot?.mfaRequireJellyfinLogin ?? false,
  };
}

async function computeAuthBootstrapSnapshot(): Promise<AuthBootstrapSnapshot> {
  let runtimeState: AuthRuntimeState | null = null;
  let effectiveFormLoginEnabled: boolean | null = null;
  let passkeyEnabled = false;
  let defaultPersistSession = false;
  let mfaRequirePasswordLogin = false;
  let mfaRequireConfigStepUp: boolean | null = null;
  let mfaRequireJellyfinLogin = false;

  try {
    const data = await queryWithRateLimitRetry<{
      authRuntimeState?: AuthRuntimeState | null;
    }>(authRuntimeStateQuery);
    runtimeState = data?.authRuntimeState ?? null;
    effectiveFormLoginEnabled =
      typeof runtimeState?.effectiveFormLoginEnabled === "boolean"
        ? runtimeState.effectiveFormLoginEnabled
        : null;
    passkeyEnabled = runtimeState?.passkeyEnabled === true;
    defaultPersistSession = runtimeState?.defaultPersistSession === true;
    mfaRequirePasswordLogin = runtimeState?.mfaRequirePasswordLogin === true;
    mfaRequireConfigStepUp =
      typeof runtimeState?.mfaRequireConfigStepUp === "boolean"
        ? runtimeState.mfaRequireConfigStepUp
        : null;
    mfaRequireJellyfinLogin = runtimeState?.mfaRequireJellyfinLogin === true;
  } catch {
    // Fall back to the existing token/bootstrap path when the public
    // runtime-state probe is temporarily unavailable.
  }

  if (runtimeState?.effectiveFormLoginEnabled === false) {
    clearPersistedAuthToken();
    return {
      token: null,
      user: await loadUserFromBypass(),
      passwordChangeRequired: false,
      effectiveFormLoginEnabled,
      passkeyEnabled,
      defaultPersistSession,
      mfaRequirePasswordLogin,
      mfaRequireConfigStepUp,
      mfaRequireJellyfinLogin,
    };
  }

  const mfaEnrollmentSnapshot = (): AuthBootstrapSnapshot => ({
    token: null,
    user: null,
    passwordChangeRequired: false,
    effectiveFormLoginEnabled,
    passkeyEnabled,
    defaultPersistSession,
    mfaRequirePasswordLogin,
    mfaRequireConfigStepUp,
    mfaRequireJellyfinLogin,
  });

  const passwordChangeRequiredSnapshot = (): AuthBootstrapSnapshot => ({
    token: null,
    user: null,
    passwordChangeRequired: true,
    effectiveFormLoginEnabled,
    passkeyEnabled,
    defaultPersistSession,
    mfaRequirePasswordLogin,
    mfaRequireConfigStepUp,
    mfaRequireJellyfinLogin,
  });

  // When auth is enabled, or the runtime mode is temporarily unknown,
  // prefer preserving an existing valid session over clearing it.
  if (currentToken) {
    if (isMfaEnrollmentToken(currentToken)) {
      return mfaEnrollmentSnapshot();
    }
    if (isPasswordChangeRequiredToken(currentToken)) {
      return passwordChangeRequiredSnapshot();
    }
    const authUser = userFromToken(currentToken);
    if (authUser) {
      return {
        token: currentToken,
        user: authUser,
        passwordChangeRequired: false,
        effectiveFormLoginEnabled,
        passkeyEnabled,
        defaultPersistSession,
        mfaRequirePasswordLogin,
        mfaRequireConfigStepUp,
        mfaRequireJellyfinLogin,
      };
    }
    clearPersistedAuthToken();
  }

  const stored = sessionStorage.getItem(SESSION_STORAGE_KEY);
  if (stored) {
    if (isMfaEnrollmentToken(stored)) {
      currentToken = stored;
      return mfaEnrollmentSnapshot();
    }
    if (isPasswordChangeRequiredToken(stored)) {
      currentToken = stored;
      return passwordChangeRequiredSnapshot();
    }
    const authUser = userFromToken(stored);
    if (authUser) {
      currentToken = stored;
      return {
        token: stored,
        user: authUser,
        passwordChangeRequired: false,
        effectiveFormLoginEnabled,
        passkeyEnabled,
        defaultPersistSession,
        mfaRequirePasswordLogin,
        mfaRequireConfigStepUp,
        mfaRequireJellyfinLogin,
      };
    }
    clearPersistedAuthToken();
  }

  let user: AuthUser | null = null;
  if (runtimeState == null || runtimeState?.skipLoginForLocalIps === true) {
    user = await loadUserFromBypass();
    if (
      !user &&
      runtimeState?.skipLoginForLocalIps === true &&
      currentToken
    ) {
      user = await loadUserFromBypass({ clearToken: true });
    }
  }

  return {
    token: null,
    user,
    passwordChangeRequired: false,
    effectiveFormLoginEnabled,
    passkeyEnabled,
    defaultPersistSession,
    mfaRequirePasswordLogin,
    mfaRequireConfigStepUp,
    mfaRequireJellyfinLogin,
  };
}

function loadAuthBootstrapSnapshot(): Promise<AuthBootstrapSnapshot> {
  if (authBootstrapSnapshot) {
    return Promise.resolve(authBootstrapSnapshot);
  }
  if (!authBootstrapPromise) {
    authBootstrapPromise = computeAuthBootstrapSnapshot().then((snapshot) => {
      authBootstrapSnapshot = snapshot;
      authBootstrapPromise = null;
      return snapshot;
    });
  }
  return authBootstrapPromise;
}

/**
 * The signed-in user, observed rather than owned.
 *
 * {@link useAuth} owns the session and the shell threads the permissions it
 * derives down as props, which is right for anything that gates real work. A
 * leaf that needs one permission for one link -- an offer to go and create more
 * tags, say -- would otherwise force that prop through every component between
 * the shell and itself, none of which has any other use for it. This reads the
 * snapshot the shell has already resolved, so it costs no request, and follows
 * the session across a sign-in or sign-out.
 */
export function useSessionUser(): AuthUser | null {
  const [user, setUser] = useState<AuthUser | null>(
    () => authBootstrapSnapshot?.user ?? null,
  );

  useEffect(() => {
    let cancelled = false;
    const readSession = () => {
      void loadAuthBootstrapSnapshot().then((snapshot) => {
        if (!cancelled) {
          setUser(snapshot.user);
        }
      });
    };

    readSession();
    window.addEventListener(AUTH_SESSION_CHANGED_EVENT, readSession);
    return () => {
      cancelled = true;
      window.removeEventListener(AUTH_SESSION_CHANGED_EVENT, readSession);
    };
  }, []);

  return user;
}

function applyAuthenticatedSession(
  token: string,
  user: AuthUser | null,
  setToken: (value: string | null) => void,
  setUser: (value: AuthUser | null) => void,
  setPasswordChangeRequired: (value: boolean) => void,
  persistSession: boolean,
) {
  const passwordChangeRequired = isPasswordChangeRequiredToken(token);
  persistAuthToken(token, passwordChangeRequired ? false : persistSession);
  setToken(passwordChangeRequired ? null : token);
  setUser(isRestrictedAuthToken(token) ? null : user);
  setPasswordChangeRequired(passwordChangeRequired);
}

export type AuthState = {
  token: string | null;
  user: AuthUser | null;
  passwordChangeRequired: boolean;
  loading: boolean;
  effectiveFormLoginEnabled: boolean | null;
  passkeyEnabled: boolean;
  defaultPersistSession: boolean;
  mfaRequirePasswordLogin: boolean;
  mfaRequireConfigStepUp: boolean | null;
  mfaRequireJellyfinLogin: boolean;
  login: (
    username: string,
    password: string,
    options?: AuthLoginOptions,
  ) => Promise<AuthLoginResult>;
  adoptSession: (
    token: string,
    user: AuthUser | null,
    persistSession?: boolean,
  ) => void;
  logout: () => void;
};

/** Extract AuthUser from a JWT payload, or null if the token is invalid/expired. */
function userFromToken(token: string): AuthUser | null {
  const payload = decodeJwtPayload(token);
  if (!payload || isTokenExpired(payload) || isRestrictedAuthScope(payload.authScope)) {
    return null;
  }
  const authorization = normalizeJwtPermissionClaims(
    payload.appPermissions,
    payload.libraryPermissions,
  );
  return {
    id: payload.sub,
    username: payload.username,
    ...authorization,
  };
}

function isMfaEnrollmentToken(token: string): boolean {
  const payload = decodeJwtPayload(token);
  return Boolean(payload && !isTokenExpired(payload) && payload.authScope === "mfa_enrollment");
}

function isPasswordChangeRequiredToken(token: string): boolean {
  const payload = decodeJwtPayload(token);
  return Boolean(
    payload &&
      !isTokenExpired(payload) &&
      payload.authScope === "password_change_required",
  );
}

function isRestrictedAuthToken(token: string): boolean {
  return isMfaEnrollmentToken(token) || isPasswordChangeRequiredToken(token);
}

function isRestrictedAuthScope(scope: unknown): boolean {
  return scope === "mfa_enrollment" || scope === "password_change_required";
}

export function useAuth(): AuthState {
  const [token, setToken] = useState<string | null>(null);
  const [user, setUser] = useState<AuthUser | null>(null);
  const [passwordChangeRequired, setPasswordChangeRequired] = useState(false);
  const [loading, setLoading] = useState(true);
  const [effectiveFormLoginEnabled, setEffectiveFormLoginEnabled] = useState<boolean | null>(null);
  const [passkeyEnabled, setPasskeyEnabled] = useState(false);
  const [defaultPersistSession, setDefaultPersistSession] = useState(false);
  const [mfaRequirePasswordLogin, setMfaRequirePasswordLogin] = useState(false);
  const [mfaRequireConfigStepUp, setMfaRequireConfigStepUp] = useState<
    boolean | null
  >(null);
  const [mfaRequireJellyfinLogin, setTotpRequireJellyfinLogin] = useState(false);
  const initialized = useRef(false);

  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;

    let cancelled = false;
    void loadAuthBootstrapSnapshot().then((snapshot) => {
      if (cancelled) {
        return;
      }
      setToken(snapshot.token);
      setUser(snapshot.user);
      setPasswordChangeRequired(snapshot.passwordChangeRequired);
      setEffectiveFormLoginEnabled(snapshot.effectiveFormLoginEnabled);
      setPasskeyEnabled(snapshot.passkeyEnabled);
      setDefaultPersistSession(snapshot.defaultPersistSession);
      setMfaRequirePasswordLogin(snapshot.mfaRequirePasswordLogin);
      setMfaRequireConfigStepUp(snapshot.mfaRequireConfigStepUp);
      setTotpRequireJellyfinLogin(snapshot.mfaRequireJellyfinLogin);
      setLoading(false);
    });

    return () => {
      cancelled = true;
      initialized.current = false;
    };
  }, []);

  const login = useCallback(async (
    username: string,
    password: string,
    options?: AuthLoginOptions,
  ) => {
    const { data, error } = await backendClient.mutation(loginMutation, {
      input: {
        username,
        password,
        totpCode: options?.totpCode ?? null,
        persistSession: options?.persistSession,
      },
    }).toPromise();
    if (error || !data?.login) {
      throw error ?? new Error("Login failed");
    }
    const newToken = data.login.token;
    const nextUser = normalizeAuthUser(data.login.user) ?? userFromToken(newToken);

    const persistSession = data.login.persistSession === true;
    applyAuthenticatedSession(
      newToken,
      nextUser,
      setToken,
      setUser,
      setPasswordChangeRequired,
      persistSession,
    );
    rememberAuthBootstrapSession(newToken, nextUser);

    return {
      token: newToken,
      user: nextUser,
      mfaEnrollmentRequired: data.login.mfaEnrollmentRequired === true,
      passwordChangeRequired: data.login.passwordChangeRequired === true,
      persistSession,
    };
  }, []);

  const adoptSession = useCallback(
    (
      nextToken: string,
      nextUser: AuthUser | null,
      persistSession = false,
    ) => {
      const normalizedUser = normalizeAuthUser(nextUser) ?? userFromToken(nextToken);
      applyAuthenticatedSession(
        nextToken,
        normalizedUser,
        setToken,
        setUser,
        setPasswordChangeRequired,
        persistSession,
      );
      rememberAuthBootstrapSession(nextToken, normalizedUser);
    },
    [],
  );

  const logout = useCallback(() => {
    clearPersistedAuthToken();
    setToken(null);
    setUser(null);
    setPasswordChangeRequired(false);
    rememberAuthBootstrapSession(null, null);
  }, []);

  return {
    token,
    user,
    passwordChangeRequired,
    loading,
    effectiveFormLoginEnabled,
    passkeyEnabled,
    defaultPersistSession,
    mfaRequirePasswordLogin,
    mfaRequireConfigStepUp,
    mfaRequireJellyfinLogin,
    login,
    adoptSession,
    logout,
  };
}
