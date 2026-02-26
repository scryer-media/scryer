
import { useCallback, useEffect, useRef, useState } from "react";
import { backendClient } from "@/lib/graphql/urql-client";
import { decodeJwtPayload, isTokenExpired } from "@/lib/utils/jwt";

const LOGIN_MUTATION = `mutation Login($input: LoginInput!) {
  login(input: $input) { token expiresAt }
}`;
const DEV_AUTO_LOGIN_MUTATION = `mutation DevAutoLogin { devAutoLogin { token expiresAt } }`;

const SESSION_STORAGE_KEY = "scryer_auth_token";

type AuthUser = { id: string; username: string; entitlements: string[] };

// Module-level token ref so getAuthToken() can be called outside React
let currentToken: string | null = null;

export function getAuthToken(): string | null {
  return currentToken;
}

export type AuthState = {
  token: string | null;
  user: AuthUser | null;
  loading: boolean;
  devMode: boolean;
  login: (username: string, password: string) => Promise<void>;
  logout: () => void;
};

/** Extract AuthUser from a JWT payload, or null if the token is invalid/expired. */
function userFromToken(token: string): AuthUser | null {
  const payload = decodeJwtPayload(token);
  if (!payload || isTokenExpired(payload)) return null;
  return {
    id: payload.sub,
    username: payload.username,
    entitlements: payload.entitlements,
  };
}

export function useAuth(): AuthState {
  const [token, setToken] = useState<string | null>(null);
  const [user, setUser] = useState<AuthUser | null>(null);
  const [loading, setLoading] = useState(true);
  const [devMode, setDevMode] = useState(false);
  const initialized = useRef(false);

  useEffect(() => {
    if (initialized.current) return;
    initialized.current = true;

    (async () => {
      // 1. Try module-level token (same SPA session)
      if (currentToken) {
        const authUser = userFromToken(currentToken);
        if (authUser) {
          setToken(currentToken);
          setUser(authUser);
          setLoading(false);
          return;
        }
        currentToken = null;
      }

      // 2. Check sessionStorage for a persisted token
      const stored = sessionStorage.getItem(SESSION_STORAGE_KEY);
      if (stored) {
        const authUser = userFromToken(stored);
        if (authUser) {
          currentToken = stored;
          setToken(stored);
          setUser(authUser);
          setLoading(false);
          return;
        }
        sessionStorage.removeItem(SESSION_STORAGE_KEY);
      }

      // 3. Dev mode probe: request an auto-login JWT (only works when SCRYER_DEV_AUTO_LOGIN=true)
      try {
        const { data } = await backendClient.mutation(DEV_AUTO_LOGIN_MUTATION, {}).toPromise();
        if (data?.devAutoLogin?.token) {
          const devToken = data.devAutoLogin.token;
          const authUser = userFromToken(devToken);
          if (authUser) {
            sessionStorage.setItem(SESSION_STORAGE_KEY, devToken);
            currentToken = devToken;
            setToken(devToken);
            setUser(authUser);
            setDevMode(true);
            setLoading(false);
            return;
          }
        }
      } catch {
        // Expected in production mode (dev auto-login not enabled)
      }

      setLoading(false);
    })();
  }, []);

  const login = useCallback(async (username: string, password: string) => {
    const { data, error } = await backendClient.mutation(LOGIN_MUTATION, {
      input: { username, password },
    }).toPromise();
    if (error || !data?.login) {
      throw error ?? new Error("Login failed");
    }
    const newToken = data.login.token;
    sessionStorage.setItem(SESSION_STORAGE_KEY, newToken);
    currentToken = newToken;
    setToken(newToken);

    const authUser = userFromToken(newToken);
    if (authUser) {
      setUser(authUser);
    }
  }, []);

  const logout = useCallback(() => {
    sessionStorage.removeItem(SESSION_STORAGE_KEY);
    currentToken = null;
    setToken(null);
    setUser(null);
  }, []);

  return { token, user, loading, devMode, login, logout };
}
