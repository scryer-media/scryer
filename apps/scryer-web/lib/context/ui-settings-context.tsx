import { userFacingGraphQlErrorMessage } from "../graphql/error-message.ts";
import * as React from "react";
import { useTheme } from "next-themes";
import { useClient, type OperationResult } from "urql";
import { setMyUiSettingsMutation } from "@/lib/graphql/mutations";
import { myUiSettingsQuery } from "@/lib/graphql/queries";
import { AUTH_SESSION_CHANGED_EVENT, getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import {
  applyHighlightColor,
  fromUiThemeValue,
  isDarkTheme,
  toUiThemeValue,
} from "@/lib/theme";
import { effectiveTheme, normalizePalette } from "@/lib/palettes";
import type { SetMyUiSettingsInput, UiSettings } from "@/lib/types/settings";

export const DEFAULT_UI_SETTINGS: UiSettings = {
  theme: "DARK",
  palette: "CLASSIC",
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
  dashboardLayouts: [],
};

type AppearanceChange = Partial<
  Pick<UiSettings, "palette" | "theme" | "highlightColor">
>;

type UiSettingsContextValue = {
  appearanceSettings: UiSettings;
  appearanceSaving: boolean;
  changeAppearance: (
    change: AppearanceChange,
  ) => Promise<"saved" | "failed" | "cancelled">;
  uiSettings: UiSettings;
  uiSettingsLoading: boolean;
  uiSettingsLoaded: boolean;
  uiSettingsLoadError: string | null;
  setUiSettings: (settings: UiSettings) => void;
  refreshUiSettings: () => Promise<void>;
  saveUiSettings: (
    build: (latest: UiSettings) => SetMyUiSettingsInput,
  ) => Promise<UiSettingsSaveResult>;
};

export type UiSettingsSaveResult = OperationResult<
  { setMyUiSettings?: UiSettings },
  { input: SetMyUiSettingsInput }
>;

export const UiSettingsContext =
  React.createContext<UiSettingsContextValue | null>(null);

function normalizeUiSettings(
  settings: Partial<UiSettings> | null | undefined,
): UiSettings {
  return {
    ...DEFAULT_UI_SETTINGS,
    ...settings,
    theme: toUiThemeValue(fromUiThemeValue(settings?.theme)),
    palette: normalizePalette(settings?.palette),
    language: settings?.language ?? null,
    tableColumns: settings?.tableColumns ?? [],
    catalogViews: settings?.catalogViews ?? [],
    dashboardLayouts: settings?.dashboardLayouts ?? [],
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
  return userFacingGraphQlErrorMessage(error, "Failed to load UI settings");
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
    palette: settings.palette,
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

export function UiSettingsProvider({
  children,
}: {
  children: React.ReactNode;
}) {
  const client = useClient();
  const { resolvedTheme, theme, setTheme } = useTheme();
  const [storedSettings, setUiSettingsState] =
    React.useState<UiSettings>(DEFAULT_UI_SETTINGS);
  const [appearancePreview, setAppearancePreview] =
    React.useState<AppearanceChange | null>(null);
  const appearancePendingRef = React.useRef(false);
  const uiSettings = React.useMemo(
    () => ({ ...storedSettings, ...appearancePreview }),
    [storedSettings, appearancePreview],
  );
  // The settings as last set, ahead of the render that shows them, so a
  // queued save builds its input from every change made before it ran.
  const latestUiSettingsRef = React.useRef<UiSettings>(DEFAULT_UI_SETTINGS);
  const setUiSettings = React.useCallback((settings: UiSettings) => {
    latestUiSettingsRef.current = settings;
    setUiSettingsState(settings);
  }, []);
  const saveQueueRef = React.useRef<Promise<void> | null>(null);
  const [uiSettingsLoading, setUiSettingsLoading] = React.useState(true);
  const [uiSettingsLoaded, setUiSettingsLoaded] = React.useState(false);
  const [uiSettingsLoadError, setUiSettingsLoadError] = React.useState<
    string | null
  >(null);
  const requestSequenceRef = React.useRef(0);
  const sessionSequenceRef = React.useRef(0);

  const loadUiSettings = React.useCallback(
    async (options?: { resetToFallback?: boolean }) => {
      const requestId = requestSequenceRef.current + 1;
      requestSequenceRef.current = requestId;

      if (options?.resetToFallback) {
        sessionSequenceRef.current += 1;
        setUiSettings(DEFAULT_UI_SETTINGS);
        setAppearancePreview(null);
        appearancePendingRef.current = false;
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
          .query<{ myUiSettings: UiSettings }>(
            myUiSettingsQuery,
            {},
            { requestPolicy: "network-only" },
          )
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
    },
    [client, setUiSettings],
  );

  const refreshUiSettings = React.useCallback(
    () => loadUiSettings(),
    [loadUiSettings],
  );

  // setMyUiSettings replaces every field it is given, so saves go out one
  // at a time and each builds its input once the saves before it settled:
  // a later save never resends a field an earlier one is still changing.
  const saveUiSettings = React.useCallback(
    (
      build: (latest: UiSettings) => SetMyUiSettingsInput,
    ): Promise<UiSettingsSaveResult> => {
      const session = sessionSequenceRef.current;
      const send = () => {
        if (session !== sessionSequenceRef.current) {
          return Promise.reject(
            new DOMException("Session changed", "AbortError"),
          );
        }
        return client
          .mutation<
            { setMyUiSettings?: UiSettings },
            { input: SetMyUiSettingsInput }
          >(setMyUiSettingsMutation, {
            input: build(latestUiSettingsRef.current),
          })
          .toPromise();
      };
      const previous = saveQueueRef.current;
      const run = previous ? previous.then(send) : send();
      const tail = run.then(
        () => undefined,
        () => undefined,
      );
      saveQueueRef.current = tail;
      void tail.then(() => {
        if (saveQueueRef.current === tail) {
          saveQueueRef.current = null;
        }
      });
      return run;
    },
    [client],
  );

  const changeAppearance = React.useCallback(
    async (
      change: AppearanceChange,
    ): Promise<"saved" | "failed" | "cancelled"> => {
      if (
        !uiSettingsLoaded ||
        uiSettingsLoading ||
        appearancePendingRef.current
      )
        return "cancelled";
      const session = sessionSequenceRef.current;
      appearancePendingRef.current = true;
      setAppearancePreview(change);
      try {
        const result = await saveUiSettings((latest) =>
          uiSettingsInputFromSettings({ ...latest, ...change }),
        );
        if (session !== sessionSequenceRef.current) return "cancelled";
        if (result.error || !result.data?.setMyUiSettings) return "failed";
        setUiSettings(result.data.setMyUiSettings);
        return "saved";
      } catch {
        return session === sessionSequenceRef.current ? "failed" : "cancelled";
      } finally {
        if (session === sessionSequenceRef.current) {
          appearancePendingRef.current = false;
          setAppearancePreview(null);
        }
      }
    },
    [saveUiSettings, setUiSettings, uiSettingsLoaded, uiSettingsLoading],
  );

  React.useEffect(() => {
    if (typeof window === "undefined") {
      void loadUiSettings({ resetToFallback: true });
      return undefined;
    }

    const handleAuthSessionChanged = () => {
      void loadUiSettings({ resetToFallback: true });
    };

    window.addEventListener(
      AUTH_SESSION_CHANGED_EVENT,
      handleAuthSessionChanged,
    );
    void loadUiSettings({ resetToFallback: true });
    return () => {
      requestSequenceRef.current += 1;
      sessionSequenceRef.current += 1;
      window.removeEventListener(
        AUTH_SESSION_CHANGED_EVENT,
        handleAuthSessionChanged,
      );
    };
  }, [loadUiSettings]);

  const activeTheme = effectiveTheme(uiSettings);
  React.useLayoutEffect(() => {
    document.documentElement.dataset.palette = normalizePalette(
      uiSettings.palette,
    );
    if (theme !== activeTheme) setTheme(activeTheme);
  }, [uiSettings.palette, activeTheme, theme, setTheme]);

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

  const value = React.useMemo<UiSettingsContextValue>(() => {
    const session = sessionSequenceRef.current;
    return {
      uiSettings: storedSettings,
      appearanceSettings: uiSettings,
      appearanceSaving: appearancePreview !== null,
      changeAppearance,
      uiSettingsLoading,
      uiSettingsLoaded,
      uiSettingsLoadError,
      // Other preference editors restore whole snapshots on failure. Only
      // changeAppearance owns appearance fields, so stale snapshots cannot
      // roll back a palette or accent saved while another editor was pending.
      setUiSettings: (settings) => {
        if (session !== sessionSequenceRef.current) return;
        const latest = latestUiSettingsRef.current;
        setUiSettings({
          ...settings,
          palette: latest.palette,
          theme: latest.theme,
          highlightColor: latest.highlightColor,
        });
      },
      refreshUiSettings,
      saveUiSettings,
    };
  }, [
    refreshUiSettings,
    saveUiSettings,
    appearancePreview,
    storedSettings,
    changeAppearance,
    setUiSettings,
    uiSettings,
    uiSettingsLoaded,
    uiSettingsLoadError,
    uiSettingsLoading,
  ]);

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
