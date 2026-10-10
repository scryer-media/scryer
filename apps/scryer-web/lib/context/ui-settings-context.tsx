import * as React from "react";
import { useTheme } from "next-themes";
import { useClient } from "urql";
import { myUiSettingsQuery } from "@/lib/graphql/queries";
import { AUTH_SESSION_CHANGED_EVENT, getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { applyHighlightColor, fromUiThemeValue, isDarkTheme, toUiThemeValue } from "@/lib/theme";
import type { SetMyUiSettingsInput, UiSettings } from "@/lib/types/settings";

export const DEFAULT_UI_SETTINGS: UiSettings = {
  theme: "DARK",
  dateTimeFormat: "LOCALE",
  highlightColor: null,
  secondaryColor: null,
  highContrastMode: false,
  reduceMotion: false,
  hideSponsorButton: false,
  density: "COMFORTABLE",
  sidebarMode: "EXPANDED",
  defaultLandingView: "MOVIES",
  language: null,
  tableColumns: [],
  catalogViews: [],
};

type UiSettingsContextValue = {
  uiSettings: UiSettings;
  uiSettingsLoading: boolean;
  uiSettingsLoaded: boolean;
  uiSettingsLoadError: string | null;
  setUiSettings: (settings: UiSettings) => void;
  refreshUiSettings: () => Promise<void>;
};

export const UiSettingsContext = React.createContext<UiSettingsContextValue | null>(null);

function normalizeUiSettings(settings: Partial<UiSettings> | null | undefined): UiSettings {
  return {
    ...DEFAULT_UI_SETTINGS,
    ...settings,
    theme: toUiThemeValue(fromUiThemeValue(settings?.theme)),
    language: settings?.language ?? null,
    tableColumns: settings?.tableColumns ?? [],
    catalogViews: settings?.catalogViews ?? [],
  };
}

/**
 * The login page before sign-in has no profile to read: user-scoped settings
 * are not requested there and their defaults are authoritative.
 */
export function isSignedOutLoginSurface(): boolean {
  if (typeof window === "undefined" || getAuthToken() !== null) {
    return false;
  }
  const basePath = getRuntimeBasePath();
  const loginPath = basePath === "/" ? "/login" : `${basePath}/login`;
  return window.location.pathname.startsWith(loginPath);
}

function uiSettingsErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : "Failed to load UI settings";
}

/**
 * Builds the whole-settings save. The language is sent only when `language`
 * is given, and table columns never are: catalog layouts save through
 * setMyCatalogView, one device class and facet at a time.
 */
export function uiSettingsInputFromSettings(
  settings: UiSettings,
  language?: string,
): SetMyUiSettingsInput {
  return {
    ...(language === undefined ? {} : { language }),
    theme: toUiThemeValue(fromUiThemeValue(settings.theme)),
    dateTimeFormat: settings.dateTimeFormat,
    highlightColor: settings.highlightColor,
    secondaryColor: settings.secondaryColor,
    highContrastMode: settings.highContrastMode,
    reduceMotion: settings.reduceMotion,
    hideSponsorButton: settings.hideSponsorButton,
    density: settings.density,
    sidebarMode: settings.sidebarMode,
    defaultLandingView: settings.defaultLandingView,
  };
}

export function UiSettingsProvider({ children }: { children: React.ReactNode }) {
  const client = useClient();
  const { resolvedTheme, theme } = useTheme();
  const [uiSettings, setUiSettings] = React.useState<UiSettings>(DEFAULT_UI_SETTINGS);
  const [uiSettingsLoading, setUiSettingsLoading] = React.useState(true);
  const [uiSettingsLoaded, setUiSettingsLoaded] = React.useState(false);
  const [uiSettingsLoadError, setUiSettingsLoadError] = React.useState<string | null>(null);
  const requestSequenceRef = React.useRef(0);

  const loadUiSettings = React.useCallback(async (options?: { resetToFallback?: boolean }) => {
    const requestId = requestSequenceRef.current + 1;
    requestSequenceRef.current = requestId;

    if (options?.resetToFallback) {
      setUiSettings(DEFAULT_UI_SETTINGS);
    }
    // On the login surface with no session, the defaults are authoritative:
    // firing the user-scoped query there turns every auth-session flap into a
    // rejected-request storm that can exhaust the origin connection pool and
    // starve the login page's own bootstrap queries.
    if (isSignedOutLoginSurface()) {
      setUiSettingsLoading(false);
      setUiSettingsLoaded(false);
      setUiSettingsLoadError(null);
      return;
    }
    setUiSettingsLoading(true);
    setUiSettingsLoaded(false);
    setUiSettingsLoadError(null);
    try {
      const { data, error } = await client
        .query<{ myUiSettings: UiSettings }>(myUiSettingsQuery, {})
        .toPromise();
      if (error) throw error;
      if (requestSequenceRef.current !== requestId) return;
      setUiSettings(normalizeUiSettings(data?.myUiSettings));
      setUiSettingsLoaded(true);
    } catch (error) {
      if (requestSequenceRef.current !== requestId) return;
      setUiSettings(DEFAULT_UI_SETTINGS);
      setUiSettingsLoaded(false);
      setUiSettingsLoadError(uiSettingsErrorMessage(error));
    } finally {
      if (requestSequenceRef.current === requestId) {
        setUiSettingsLoading(false);
      }
    }
  }, [client]);

  const refreshUiSettings = React.useCallback(
    () => loadUiSettings(),
    [loadUiSettings],
  );

  React.useEffect(() => {
    if (typeof window === "undefined") {
      void loadUiSettings({ resetToFallback: true });
      return undefined;
    }

    const handleAuthSessionChanged = () => {
      void loadUiSettings({ resetToFallback: true });
    };

    window.addEventListener(AUTH_SESSION_CHANGED_EVENT, handleAuthSessionChanged);
    void loadUiSettings({ resetToFallback: true });
    return () => {
      requestSequenceRef.current += 1;
      window.removeEventListener(AUTH_SESSION_CHANGED_EVENT, handleAuthSessionChanged);
    };
  }, [loadUiSettings]);

  // Apply the user's highlight color to the accent CSS variables on bootstrap,
  // whenever it changes (live), and when the active theme flips (so on-accent
  // text/border shades track the light vs dark background).
  React.useEffect(() => {
    if (typeof document === "undefined") return;
    applyHighlightColor(
      document.documentElement,
      uiSettings.highlightColor,
      isDarkTheme(resolvedTheme ?? theme),
    );
  }, [uiSettings.highlightColor, resolvedTheme, theme]);

  const value = React.useMemo<UiSettingsContextValue>(
    () => ({
      uiSettings,
      uiSettingsLoading,
      uiSettingsLoaded,
      uiSettingsLoadError,
      setUiSettings,
      refreshUiSettings,
    }),
    [
      refreshUiSettings,
      uiSettings,
      uiSettingsLoaded,
      uiSettingsLoadError,
      uiSettingsLoading,
    ],
  );

  return (
    <UiSettingsContext.Provider value={value}>
      {children}
    </UiSettingsContext.Provider>
  );
}

export function useUiSettings() {
  const value = React.useContext(UiSettingsContext);
  if (!value) {
    throw new Error("useUiSettings must be used within UiSettingsProvider");
  }
  return value;
}

export function useUiDateTimeFormat() {
  return useUiSettings().uiSettings.dateTimeFormat;
}
