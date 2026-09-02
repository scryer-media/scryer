import {
  lazy,
  Suspense,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import {
  ActivitySquare,
  AlertOctagon,
  AlertTriangle,
  CalendarDays,
  Download,
  LayoutDashboard,
  ListChecks,
  Loader2,
  Monitor,
  Settings,
  CircleFadingArrowUp,
  Sparkles,
  WifiOff,
  X,
} from "lucide-react";
import { useLocation, useNavigate, useSearchParams } from "react-router";
import { useAuth, type AuthUser } from "@/lib/hooks/use-auth";
import { usePermissions } from "@/lib/hooks/use-permissions";
import {
  APP_PERMISSIONS,
  LIBRARY_PERMISSIONS,
  authorizationCacheSignature,
  hasAnyLibraryPermission,
  hasAppPermission,
  hasLibraryPermission,
} from "@/lib/utils/permissions";
import { useSmgNotices } from "@/lib/hooks/use-smg-notices";
import { useNavigationBadges } from "@/lib/hooks/use-navigation-badges";
import { useAutoBackupNotice } from "@/lib/hooks/use-auto-backup-notice";
import { useConfigStepUp } from "@/lib/hooks/use-config-step-up";

import { TranslateContext } from "@/lib/context/translate-context";
import { GlobalStatusContext } from "@/lib/context/global-status-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { RootHeader } from "@/components/root/root-header";
import { buildRouteCommands } from "@/components/root/route-commands";
import { JobRunProvider } from "@/components/root/job-run-provider";
import { LibraryScanProgressProvider } from "@/components/root/library-scan-progress-provider";
import { ReactiveRefreshProvider } from "@/components/root/reactive-refresh-provider";
import { RootSidebar } from "@/components/root/root-sidebar";
import { ApplicationUpgradeAction } from "@/components/common/application-upgrade";
import { ViewLoadingFallback } from "@/components/common/view-loading-fallback";
import { GlobalSearchProvider } from "@/components/root/global-search-provider";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";

import { useGlobalStatusToast } from "@/lib/hooks/use-global-status-toast";
import { useLanguage } from "@/lib/hooks/use-language";
import { ScryerGraphqlProvider } from "@/lib/graphql/urql-provider";
import { backendClient } from "@/lib/graphql/urql-client";
import { useOnlineStatus } from "@/lib/hooks/use-online-status";
import { useInstallPrompt } from "@/lib/hooks/use-install-prompt";
import { useBackendRestarting } from "@/lib/hooks/use-backend-restarting";
import { useApplicationUpgradeStatus } from "@/lib/hooks/use-application-upgrade-status";
import { TotpCodeForm } from "@/components/auth/totp-code-form";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type {
  ActivitySection,
  ViewId,
  SettingsSection,
  ContentSettingsSection,
  LogsSection,
  OverviewTitleTarget,
  SmgScryerUpdateNotice,
  SmgVersionCompatibilityNotice,
  SystemSection,
  WantedSection,
} from "@/components/root/types";
import type { Facet } from "@/lib/types";
import type { UiDateTimeFormat } from "@/lib/types/settings";
import { formatUiDate } from "@/lib/utils/date-format";
import {
  URL_PARAM_CONTENT_SECTION_DEPRECATED,
  URL_PARAM_LANGUAGE,
  URL_PARAM_SETTINGS_SECTION_DEPRECATED,
  URL_PARAM_VIEW_DEPRECATED,
} from "@/lib/constants/settings";
import { AVAILABLE_LANGUAGES } from "@/lib/i18n";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";

import {
  buildOverviewDetailPath,
  buildViewPath,
  resolveAppRoute,
} from "@/lib/utils/routing";
import {
  canAccessDashboard,
  canAccessMediaSettingsSection,
  canAccessSettingsSection,
  canAccessSystemSection,
  defaultAccessibleRoute,
  defaultSettingsSection,
  isMediaSettingsSection,
  isProtectedSettingsRoute,
} from "@/lib/utils/routes";
import { cn } from "@/lib/utils";
import {
  FACET_REGISTRY,
  isMediaView,
  facetForView,
} from "@/lib/facets/registry";
import { BackendRestartOverlay } from "@/components/common/backend-restart-overlay";
import {
  resolveTitleOverviewTargetById,
  resolveTitleOverviewTargetBySlug,
} from "@/lib/title-overview-loader";

const DashboardContainer = lazy(() =>
  import("@/components/containers/dashboard-container").then((m) => ({
    default: m.DashboardContainer,
  })),
);

const MediaContentContainer = lazy(() =>
  import("@/components/containers/media-content-container").then((m) => ({
    default: m.MediaContentContainer,
  })),
);

const RequestsContainer = lazy(() =>
  import("@/components/containers/requests-container").then((m) => ({
    default: m.RequestsContainer,
  })),
);

const SettingsContainer = lazy(() =>
  import("@/components/containers/settings/settings-container").then((m) => ({
    default: m.SettingsContainer,
  })),
);

const NotFoundPage = lazy(() => import("@/src/pages/not-found"));

const ActivityContainer = lazy(() =>
  import("@/components/containers/activity-container").then((m) => ({
    default: m.ActivityContainer,
  })),
);

const TitleHistoryContainer = lazy(() =>
  import("@/components/containers/title-history-container").then((m) => ({
    default: m.TitleHistoryContainer,
  })),
);

const SystemContainer = lazy(() =>
  import("@/components/containers/system-container").then((m) => ({
    default: m.SystemContainer,
  })),
);

const WantedContainer = lazy(() =>
  import("@/components/containers/wanted-container").then((m) => ({
    default: m.WantedContainer,
  })),
);

const DiscoveryContainer = lazy(() =>
  import("@/components/containers/discovery-container").then((m) => ({
    default: m.DiscoveryContainer,
  })),
);

const CalendarContainer = lazy(() =>
  import("@/components/containers/calendar-container").then((m) => ({
    default: m.CalendarContainer,
  })),
);

const PendingImportsContainer = lazy(() =>
  import("@/components/containers/pending-imports-container").then((m) => ({
    default: m.PendingImportsContainer,
  })),
);

const INSTALL_BANNER_DISMISSED_KEY = "scryer.pwa.installBannerDismissed";

type TranslateFn = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

function normalizeSmgVersionCompatibilityStatus(
  status: string,
): "deprecated" | "blocked" {
  return status.trim().toLowerCase() === "deprecated"
    ? "deprecated"
    : "blocked";
}

function fallbackMediaContentSettingsSection(
  section: ContentSettingsSection,
  canManageConfig: boolean,
  canManageLibrarySettings: boolean,
  canResolveImports = false,
): ContentSettingsSection {
  if (
    canManageLibrarySettings &&
    !canManageConfig &&
    isMediaSettingsSection(section)
  ) {
    return "library";
  }
  if (canResolveImports) {
    return "import";
  }
  return "overview";
}


function formatSmgUpgradeDeadline(
  value: string | null,
  dateTimeFormat: UiDateTimeFormat,
): string | null {
  if (!value) {
    return null;
  }

  return formatUiDate(value, dateTimeFormat, { fallback: value });
}

function SmgUpgradeBanner({
  notice,
  t,
}: {
  notice: SmgVersionCompatibilityNotice;
  t: TranslateFn;
}) {
  const dateTimeFormat = useUiDateTimeFormat();
  const status = normalizeSmgVersionCompatibilityStatus(notice.status);
  const isDeprecated = status === "deprecated";
  const Icon = isDeprecated ? AlertTriangle : AlertOctagon;
  const deadline = formatSmgUpgradeDeadline(notice.upgradeDeadline, dateTimeFormat);
  const minimumVersion = notice.minimumVersion.trim();
  const serverMessage = notice.message.trim();
  const details = [
    minimumVersion
      ? t("smgUpgrade.minimumVersion", { version: minimumVersion })
      : null,
    deadline ? t("smgUpgrade.deadline", { date: deadline }) : null,
  ].filter((value): value is string => Boolean(value));

  return (
    <div
      data-slot="root-shell-notice"
      className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-surfA))] text-[var(--scry-body)] shadow-[0_8px_28px_rgba(2,6,23,0.14)] backdrop-blur"
    >
      <div className="mx-auto flex w-full max-w-[1480px] items-start gap-3 px-4 py-3">
        <span
          className={cn(
            "mt-0.5 flex h-8 w-8 flex-none items-center justify-center rounded-[10px] border shadow-[0_8px_20px_rgba(2,6,23,0.10)]",
            isDeprecated
              ? "border-[rgba(var(--scry-accent-rgb),0.28)] bg-[rgba(var(--scry-accent-rgb),0.14)] text-[var(--scry-accent-ring)]"
              : "border-destructive/30 bg-destructive/10 text-destructive",
          )}
        >
          <Icon className="h-4 w-4" aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <div className="font-semibold text-[var(--scry-ink2)]">
            {isDeprecated
              ? t("smgUpgrade.deprecatedTitle")
              : t("smgUpgrade.blockedTitle")}
          </div>
          <div className="mt-0.5 text-sm text-[var(--scry-muted)]">
            {isDeprecated
              ? t("smgUpgrade.deprecatedBody")
              : t("smgUpgrade.blockedBody")}
          </div>
          {serverMessage ? (
            <div className="mt-1 text-sm text-[var(--scry-body)]">
              {serverMessage}
            </div>
          ) : null}
          {details.length > 0 ? (
            <div className="mt-1 text-xs font-medium uppercase tracking-wide text-[var(--scry-muted2)]">
              {details.join(" • ")}
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function SmgScryerUpdateBanner({
  canManageSystemSettings,
  notice,
  t,
  onDismiss,
}: {
  canManageSystemSettings: boolean;
  notice: SmgScryerUpdateNotice;
  t: TranslateFn;
  onDismiss: () => void;
}) {
  const currentVersion = notice.currentVersion.trim();
  const latestVersion = notice.latestVersion.trim();
  const releaseUrl = notice.releaseUrl?.trim() || null;
  const { status, refresh } = useApplicationUpgradeStatus(canManageSystemSettings);

  return (
    <div
      data-slot="root-shell-notice"
      className="border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-surfA))] text-[var(--scry-body)] shadow-[0_8px_28px_rgba(var(--scry-accent-rgb),0.10)] backdrop-blur"
    >
      <div className="mx-auto flex w-full max-w-[1480px] items-center gap-3 px-4 py-2 text-sm">
        <CircleFadingArrowUp
          className="h-4 w-4 flex-none text-[var(--scry-accent-ring)]"
          aria-hidden="true"
        />
        <div className="min-w-0 flex-1 truncate">
          <span className="font-medium text-[var(--scry-ink2)]">
            {t("smgUpdate.title")}
          </span>
          <span className="ml-2 text-[var(--scry-muted)]">
            {t("smgUpdate.body", {
              current: currentVersion || t("label.unknown"),
              latest: latestVersion || t("label.unknown"),
            })}
          </span>
        </div>
        {releaseUrl ? (
          <a
            href={releaseUrl}
            target="_blank"
            rel="noreferrer"
            className="flex-none rounded-[8px] border border-[var(--scry-border2)] bg-[rgba(var(--scry-accent-rgb),0.12)] px-2.5 py-1 text-xs font-medium text-[var(--scry-accent-text)] transition hover:border-[var(--scry-bhover2)] hover:bg-[rgba(var(--scry-accent-rgb),0.18)]"
          >
            {t("smgUpdate.releaseNotes")}
          </a>
        ) : null}
        {status?.eligible ? (
          <ApplicationUpgradeAction
            status={status}
            className="flex-none"
            onStatusChanged={() => void refresh()}
          />
        ) : null}
        <IconButton
          type="button"
          onClick={onDismiss}
          label={t("label.dismiss")}
          appearance="ghost"
          className="h-7 w-7 flex-none rounded-[8px]"
        >
          <X className="h-4 w-4" />
        </IconButton>
      </div>
    </div>
  );
}

type OverviewNavigationState = {
  scryerOverviewTarget?: {
    view?: unknown;
    id?: unknown;
    slug?: unknown;
    libraryId?: unknown;
    librarySlug?: unknown;
  };
};

function readOverviewTargetFromLocationState(
  state: unknown,
  view: ViewId,
  parsedOverviewSlug: string | null,
  parsedOverviewLibrarySlug: string | null,
): OverviewTitleTarget | null {
  if (
    !parsedOverviewSlug ||
    !parsedOverviewLibrarySlug ||
    state == null ||
    typeof state !== "object"
  ) {
    return null;
  }

  const overviewState = state as OverviewNavigationState;
  const target = overviewState.scryerOverviewTarget;
  if (!target || target.view !== view) {
    return null;
  }

  const id = typeof target.id === "string" ? target.id.trim() : "";
  const slug = typeof target.slug === "string" ? target.slug.trim() : "";
  const libraryId =
    typeof target.libraryId === "string" ? target.libraryId.trim() : "";
  const librarySlug =
    typeof target.librarySlug === "string" ? target.librarySlug.trim() : "";
  const effectiveLibrarySlug =
    librarySlug ||
    (parsedOverviewLibrarySlug === view ? parsedOverviewLibrarySlug : "");
  if (
    !id ||
    !slug ||
    slug !== parsedOverviewSlug ||
    !effectiveLibrarySlug ||
    effectiveLibrarySlug !== parsedOverviewLibrarySlug
  ) {
    return null;
  }

  return {
    id,
    slug,
    libraryId: libraryId || null,
    librarySlug: effectiveLibrarySlug,
  };
}

/**
 * Renders the main content area.
 */
function MainContent({
  view,
  overviewTitleId,
  overviewTitleRoutePending,
  routeOverviewEpisodeId,
  handleBackToList,
  settingsSection,
  userId,
  username,
  canManageTitlesInLibrary,
  selectedLanguage,
  uiLanguage,
  discoveryAuthorizationSignature,
  setLanguagePreferenceFromShell,
  contentSettingsSection,
  systemSection,
  logsSection,
  scryerVersion,
  pluginUpdateCount,
  activitySection,
  wantedSection,
  handleOpenOverview,
  handleImportRouteEmpty,
  canViewCatalog,
  canAccessActivity,
  canResolveImports,
  canManageTitle,
  canRequestMedia,
  canManageUserAccounts,
  canManageUsers,
  canManageSystemSettings,
  canManageCatalogSettings,
  canManageConfig,
  canManageLibrarySettings,
}: {
  view: ViewId;
  overviewTitleId: string | null;
  overviewTitleRoutePending: boolean;
  routeOverviewEpisodeId: string | null;
  handleBackToList: () => void;
  settingsSection: SettingsSection;
  userId: string | undefined;
  canManageTitlesInLibrary: (libraryId: string | null | undefined) => boolean;
  username: string | undefined;
  selectedLanguage: LanguageOption;
  uiLanguage: LocaleCode;
  discoveryAuthorizationSignature: string;
  setLanguagePreferenceFromShell: (code: string) => void;
  contentSettingsSection: ContentSettingsSection;
  systemSection: SystemSection;
  logsSection: LogsSection;
  scryerVersion: string | null;
  pluginUpdateCount: number;
  activitySection: ActivitySection;
  wantedSection: WantedSection;
  handleOpenOverview: (
    targetView: ViewId,
    overviewTarget: OverviewTitleTarget,
    episodeId?: string,
  ) => void;
  handleImportRouteEmpty: () => void;
  canViewCatalog: boolean;
  canAccessActivity: boolean;
  canResolveImports: boolean;
  canManageTitle: boolean;
  canRequestMedia: boolean;
  canManageUserAccounts: boolean;
  canManageUsers: boolean;
  canManageSystemSettings: boolean;
  canManageCatalogSettings: boolean;
  canManageConfig: boolean;
  canManageLibrarySettings: boolean;
}) {
  if (view === "dashboard") {
    if (!canAccessDashboard(canManageSystemSettings)) {
      return <ViewLoadingFallback />;
    }
    return <DashboardContainer key="dashboard" />;
  }
  if (view === "activity") {
    if (!canAccessActivity) {
      return <ViewLoadingFallback />;
    }
    if (activitySection === "history") {
      return <TitleHistoryContainer key="activity-history" showRetryActions={false} />;
    }
    return (
      <ActivityContainer key={`activity-${activitySection}`} activitySection={activitySection} />
    );
  }
  if (view === "calendar") {
    return (
      <CalendarContainer key="calendar" onOpenOverview={handleOpenOverview} />
    );
  }
  if (view === "discovery") {
    return (
      <DiscoveryContainer
        key="discovery"
        userId={userId}
        uiLanguage={uiLanguage}
        authorizationSignature={discoveryAuthorizationSignature}
        canManageTitle={canManageTitle}
        canRequestMedia={canRequestMedia}
      />
    );
  }
  if (view === "requests") {
    if (!canManageTitle && !canRequestMedia) {
      return <ViewLoadingFallback />;
    }
    return <RequestsContainer key="requests" facet={null} />;
  }
  if (view === "wanted") {
    return (
      <WantedContainer
        key={`wanted-${wantedSection}`}
        wantedSection={wantedSection}
        onOpenOverview={handleOpenOverview}
      />
    );
  }
  if (view === "system") {
    if (
      !canAccessSystemSection(
        systemSection,
        canManageSystemSettings,
        canManageTitle,
      )
    ) {
      return <ViewLoadingFallback />;
    }
    return (
      <SystemContainer
        key={`system-${systemSection}`}
        systemSection={systemSection}
        scryerVersion={scryerVersion}
      />
    );
  }
  if (view === "logs") {
    if (!canManageSystemSettings) {
      return <ViewLoadingFallback />;
    }
    return (
      <SystemContainer
        key={`logs-${logsSection}`}
        systemSection={logsSection}
        scryerVersion={scryerVersion}
      />
    );
  }
  if (
    isMediaView(view) &&
    contentSettingsSection === "import" &&
    canResolveImports
  ) {
    return (
      <PendingImportsContainer
        key={`${view}-imports`}
        view={view}
        onNavigateBackToOverview={handleImportRouteEmpty}
      />
    );
  }
  if (
    isMediaView(view) &&
    contentSettingsSection === "overview" &&
    !canViewCatalog
  ) {
    return <ViewLoadingFallback />;
  }
  if (view === "settings") {
    const resolvedSettingsSection = canAccessSettingsSection(
      settingsSection,
      canManageUserAccounts,
      canManageUsers,
      canManageSystemSettings,
      canManageCatalogSettings,
    )
      ? settingsSection
      : defaultSettingsSection(
          canManageSystemSettings,
          canManageCatalogSettings,
          canManageUserAccounts,
          canManageUsers,
        );
    return (
      <SettingsContainer
        key="settings"
        settingsSection={resolvedSettingsSection}
        userId={userId}
        username={username}
        canManageSystemSettings={canManageSystemSettings}
        canManageCatalogSettings={canManageCatalogSettings}
        availableLanguages={AVAILABLE_LANGUAGES}
        selectedLanguage={selectedLanguage}
        uiLanguage={uiLanguage}
        onSelectLanguage={setLanguagePreferenceFromShell}
        pluginUpdateCount={pluginUpdateCount}
      />
    );
  }
  const effectiveContentSettingsSection = canAccessMediaSettingsSection(
    contentSettingsSection,
    canManageConfig,
    canManageLibrarySettings,
    canResolveImports,
  )
    ? contentSettingsSection
    : fallbackMediaContentSettingsSection(
        contentSettingsSection,
        canManageConfig,
        canManageLibrarySettings,
        canResolveImports,
      );
  return (
    <MediaContentContainer
      key={`${view}-${effectiveContentSettingsSection}`}
      view={view}
      contentSettingsSection={effectiveContentSettingsSection}
      canManageConfig={canManageConfig}
      canManageSystemSettings={canManageSystemSettings}
      canManageCatalogSettings={canManageCatalogSettings}
      canManageLibrarySettings={canManageLibrarySettings}
      canViewCatalog={canViewCatalog}
      canManageTitle={canManageTitle}
      canManageTitlesInLibrary={canManageTitlesInLibrary}
      canRequestMedia={canRequestMedia}
      authorizationSignature={discoveryAuthorizationSignature}
      onOpenOverview={handleOpenOverview}
      routeOverviewTitleId={overviewTitleId}
      routeOverviewPending={overviewTitleRoutePending}
      routeOverviewEpisodeId={routeOverviewEpisodeId}
      onCloseOverview={handleBackToList}
    />
  );
}

export default function HomePage() {
  const { serviceRestarting } = useBackendRestarting();
  const {
    token,
    user,
    loading: authLoading,
    effectiveFormLoginEnabled,
    mfaRequireConfigStepUp,
    adoptSession,
  } = useAuth();
  const navigate = useNavigate();
  const [setupChecked, setSetupChecked] = useState(false);

  useEffect(() => {
    if (
      !serviceRestarting &&
      !authLoading &&
      !user &&
      effectiveFormLoginEnabled === true
    ) {
      navigate("/login", { replace: true });
    }
  }, [
    authLoading,
    effectiveFormLoginEnabled,
    user,
    navigate,
    serviceRestarting,
  ]);

  // Check if setup wizard needs to run (first-run detection).
  useEffect(() => {
    if (serviceRestarting || authLoading || !user || setupChecked) return;
    (async () => {
      try {
        const { data } = await import("@/lib/graphql/urql-client").then((mod) =>
          mod.backendClient
            .query(
              `query SetupStatus { setupStatus { setupComplete } }`,
              {},
              { requestPolicy: "network-only" },
            )
            .toPromise(),
        );
        if (data?.setupStatus?.setupComplete === false) {
          navigate("/setup", { replace: true });
          return;
        }
      } catch {
        // If the query fails (e.g., old backend), skip the check
      }
      setSetupChecked(true);
    })();
  }, [authLoading, user, setupChecked, navigate, serviceRestarting]);

  if (serviceRestarting) {
    return <BackendRestartOverlay />;
  }

  if (authLoading || (!setupChecked && user)) {
    return (
      <div
        data-slot="root-app-frame"
        className="flex min-h-dvh items-center justify-center text-[var(--scry-body)]"
      >
        <Loader2 className="h-6 w-6 animate-spin text-[var(--scry-accent-ring)]" />
      </div>
    );
  }

  if (!user) {
    if (effectiveFormLoginEnabled !== true) {
      return (
        <div
          data-slot="root-app-frame"
          className="flex min-h-dvh items-center justify-center text-[var(--scry-body)]"
        >
          <Loader2 className="h-6 w-6 animate-spin text-[var(--scry-accent-ring)]" />
        </div>
      );
    }
    return null;
  }

  return (
    <AuthenticatedHomePage
      authToken={token}
      authenticatedUser={user}
      mfaRequireConfigStepUp={mfaRequireConfigStepUp}
      adoptSession={adoptSession}
      serviceRestarting={serviceRestarting}
    />
  );
}

function AuthenticatedHomePage({
  authToken,
  authenticatedUser,
  mfaRequireConfigStepUp,
  adoptSession,
  serviceRestarting,
}: {
  authToken: string | null;
  authenticatedUser: AuthUser;
  mfaRequireConfigStepUp: boolean | null;
  adoptSession: (nextToken: string, nextUser: AuthUser | null) => void;
  serviceRestarting: boolean;
}) {
  const isOnline = useOnlineStatus();
  const { canPrompt, isInstalled, isIosSafari, promptInstall } =
    useInstallPrompt();

  const location = useLocation();
  const { pathname } = location;
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();

  const routeResolution = useMemo(
    () => resolveAppRoute(pathname, location.search, location.hash),
    [location.hash, location.search, pathname],
  );
  const resolvedRoute =
    routeResolution.kind === "canonical"
      ? routeResolution.route
      : {
          canonicalPath: pathname,
          view: "movies" as ViewId,
          settingsSection: "profile" as SettingsSection,
          contentSettingsSection: "overview" as ContentSettingsSection,
          systemSection: "overview" as SystemSection,
          logsSection: "logs" as LogsSection,
          activitySection: "activity" as ActivitySection,
          wantedSection: "wanted" as WantedSection,
          overviewLibrarySlug: null,
          overviewTitleSlug: null,
        };
  const {
    view,
    settingsSection,
    contentSettingsSection,
    systemSection,
    logsSection,
    activitySection,
    wantedSection,
    overviewLibrarySlug: parsedOverviewLibrarySlug,
    overviewTitleSlug: parsedOverviewSlug,
  } = resolvedRoute;
  const routeIsCanonical = routeResolution.kind === "canonical";
  const routeIsLanding = routeResolution.kind === "landing";

  const legacyOverviewTitleId = useMemo(() => {
    if (
      !isMediaView(view) ||
      contentSettingsSection !== "overview" ||
      parsedOverviewSlug
    )
      return null;
    return searchParams.get("id")?.trim() || null;
  }, [view, contentSettingsSection, parsedOverviewSlug, searchParams]);

  const navigationOverviewTarget = useMemo(
    () =>
      readOverviewTargetFromLocationState(
        location.state,
        view,
        parsedOverviewSlug,
        parsedOverviewLibrarySlug,
      ),
    [location.state, parsedOverviewLibrarySlug, parsedOverviewSlug, view],
  );

  const overviewEpisodeId = useMemo(
    () => searchParams.get("episodeId")?.trim() || null,
    [searchParams],
  );

  const {
    uiLanguage,
    setLanguagePreference,
    selectedLanguage,
    t,
    getLanguageLabel,
  } = useLanguage(searchParams);

  const [, setGlobalStatusRaw] = useState("");
  const setGlobalStatus = useGlobalStatusToast(setGlobalStatusRaw);
  const shellFrameRef = useRef<HTMLDivElement>(null);
  const [shellTopOffset, setShellTopOffset] = useState(0);
  const canSubscribeToLibraryEvents = hasAnyLibraryPermission(
    authenticatedUser,
    LIBRARY_PERMISSIONS.view,
  );
  const canSubscribeToJobEvents = hasAppPermission(
    authenticatedUser,
    APP_PERMISSIONS.manageSystemSettings,
  );
  const {
    smgVersionCompatibilityNotice,
    smgScryerUpdateNotice,
    showSmgScryerUpdateReminder,
    dismissSmgScryerUpdateReminder,
    refreshSmgNotices,
  } = useSmgNotices({
    settingsSubscriptionEnabled: canSubscribeToLibraryEvents,
  });
  const [resolvedOverviewTarget, setResolvedOverviewTarget] =
    useState<OverviewTitleTarget | null>(null);
  const [overviewSlugLoading, setOverviewSlugLoading] = useState(false);
  const [legacyOverviewResolution, setLegacyOverviewResolution] = useState<{
    requestedId: string;
    resolvedId: string;
  } | null>(null);

  const setLanguagePreferenceFromShell = useCallback(
    (code: string) => {
      setLanguagePreference(code);
      setGlobalStatus(
        t("status.languageChanged", { language: getLanguageLabel(code) }),
      );
    },
    [getLanguageLabel, setLanguagePreference, t, setGlobalStatus],
  );

  const [installBannerDismissed, setInstallBannerDismissed] = useState(() => {
    if (typeof window === "undefined") {
      return false;
    }

    return window.localStorage.getItem(INSTALL_BANNER_DISMISSED_KEY) === "true";
  });
  const showInstallBanner =
    !isInstalled && !installBannerDismissed && (canPrompt || isIosSafari);
  useEffect(() => {
    const frame = shellFrameRef.current;
    if (!frame || typeof window === "undefined") {
      return;
    }

    let animationFrame: number | null = null;
    const updateShellOffset = () => {
      if (animationFrame !== null) {
        window.cancelAnimationFrame(animationFrame);
      }
      animationFrame = window.requestAnimationFrame(() => {
        const topOffset = Math.round(
          Math.max(0, frame.getBoundingClientRect().top),
        );
        setShellTopOffset((previousOffset) =>
          previousOffset === topOffset ? previousOffset : topOffset,
        );
        animationFrame = null;
      });
    };

    updateShellOffset();
    window.addEventListener("resize", updateShellOffset);
    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(updateShellOffset);
    observer?.observe(frame);
    if (frame.parentElement) {
      observer?.observe(frame.parentElement);
    }

    return () => {
      if (animationFrame !== null) {
        window.cancelAnimationFrame(animationFrame);
      }
      window.removeEventListener("resize", updateShellOffset);
      observer?.disconnect();
    };
  }, [
    isOnline,
    showInstallBanner,
    showSmgScryerUpdateReminder,
    smgScryerUpdateNotice,
    smgVersionCompatibilityNotice,
  ]);
  useEffect(() => {
    if (!isInstalled || typeof window === "undefined") {
      return;
    }

    window.localStorage.removeItem(INSTALL_BANNER_DISMISSED_KEY);
    setInstallBannerDismissed(false);
  }, [isInstalled]);

  const dismissInstallBanner = useCallback(() => {
    setInstallBannerDismissed(true);
    if (typeof window !== "undefined") {
      window.localStorage.setItem(INSTALL_BANNER_DISMISSED_KEY, "true");
    }
  }, []);

  const activeFacet = useMemo<Facet>(
    () => facetForView(view)?.id ?? "MOVIE",
    [view],
  );
  const queueFacet = activeFacet;

  const navigateTo = useCallback(
    (
      nextView: ViewId,
      nextSettingsSection?: SettingsSection,
      nextContentSection?: ContentSettingsSection,
      nextSystemSection?: SystemSection,
      nextWantedSection?: WantedSection,
      nextActivitySection?: ActivitySection,
      nextLogsSection?: LogsSection,
      nextOverviewTitleId?: string | null,
      nextEpisodeId?: string | null,
    ) => {
      const isMedia = isMediaView(nextView);
      const targetPath = buildViewPath(
        nextView,
        nextView === "settings" ? nextSettingsSection : undefined,
        isMedia ? nextContentSection : undefined,
        nextView === "system" ? nextSystemSection : undefined,
        nextView === "wanted" ? nextWantedSection : undefined,
        nextView === "activity" ? nextActivitySection : undefined,
        nextView === "logs" ? nextLogsSection : undefined,
      );
      const normalizedContentSection = isMedia
        ? (nextContentSection ?? "overview")
        : "overview";
      const normalizedOverviewTitleId =
        (nextOverviewTitleId ?? "").trim().length > 0
          ? (nextOverviewTitleId as string).trim()
          : null;

      const nextParams = new URLSearchParams(searchParams.toString());
      nextParams.delete(URL_PARAM_VIEW_DEPRECATED);
      nextParams.delete(URL_PARAM_SETTINGS_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_CONTENT_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_LANGUAGE);
      nextParams.delete("tab");
      if (
        normalizedOverviewTitleId &&
        isMedia &&
        normalizedContentSection === "overview"
      ) {
        nextParams.set("id", normalizedOverviewTitleId);
      } else {
        nextParams.delete("id");
      }
      if (nextEpisodeId) {
        nextParams.set("episodeId", nextEpisodeId);
      } else {
        nextParams.delete("episodeId");
      }

      const nextQuery = nextParams.toString();
      const nextPathWithQuery = `${targetPath}${nextQuery ? `?${nextQuery}` : ""}`;
      const currentPathWithQuery = `${pathname}${searchParams.toString() ? `?${searchParams.toString()}` : ""}`;

      if (nextPathWithQuery !== currentPathWithQuery) {
        navigate(nextPathWithQuery);
      }
    },
    [navigate, searchParams, pathname],
  );

  const navigateToOverview = useCallback(
    (
      targetView: ViewId,
      overviewTarget: OverviewTitleTarget,
      episodeId?: string | null,
      replace = false,
    ) => {
      if (!isMediaView(targetView)) {
        return;
      }

      const normalizedTitleId = overviewTarget.id.trim();
      if (!normalizedTitleId) {
        return;
      }

      const normalizedSlug = overviewTarget.slug?.trim() || null;
      const normalizedLibrarySlug = overviewTarget.librarySlug?.trim() || null;
      const normalizedLibraryId = overviewTarget.libraryId?.trim() || null;
      const hasSlugRoute = Boolean(normalizedSlug && normalizedLibrarySlug);
      const targetPath = buildOverviewDetailPath(
        targetView,
        normalizedLibrarySlug,
        normalizedSlug,
      );
      const nextParams = new URLSearchParams(searchParams.toString());
      nextParams.delete(URL_PARAM_VIEW_DEPRECATED);
      nextParams.delete(URL_PARAM_SETTINGS_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_CONTENT_SECTION_DEPRECATED);
      nextParams.delete(URL_PARAM_LANGUAGE);
      nextParams.delete("tab");
      if (!hasSlugRoute) {
        nextParams.set("id", normalizedTitleId);
      } else {
        nextParams.delete("id");
      }
      if (episodeId) {
        nextParams.set("episodeId", episodeId);
      } else {
        nextParams.delete("episodeId");
      }

      const nextQuery = nextParams.toString();
      const nextPathWithQuery = `${targetPath}${nextQuery ? `?${nextQuery}` : ""}`;
      const currentPathWithQuery = `${pathname}${searchParams.toString() ? `?${searchParams.toString()}` : ""}`;
      const state = hasSlugRoute
        ? {
            scryerOverviewTarget: {
              view: targetView,
              id: normalizedTitleId,
              slug: normalizedSlug,
              libraryId: normalizedLibraryId,
              librarySlug: normalizedLibrarySlug,
            },
          }
        : undefined;

      if (nextPathWithQuery !== currentPathWithQuery) {
        navigate(nextPathWithQuery, { replace, state });
      }
    },
    [navigate, pathname, searchParams],
  );

  useEffect(() => {
    let cancelled = false;

    if (navigationOverviewTarget) {
      setResolvedOverviewTarget(navigationOverviewTarget);
      setOverviewSlugLoading(false);
      return () => {
        cancelled = true;
      };
    }

    if (
      !isMediaView(view) ||
      contentSettingsSection !== "overview" ||
      !parsedOverviewLibrarySlug ||
      !parsedOverviewSlug
    ) {
      setResolvedOverviewTarget(null);
      setOverviewSlugLoading(false);
      return () => {
        cancelled = true;
      };
    }

    const facet = facetForView(view)?.id;
    if (!facet) {
      setResolvedOverviewTarget(null);
      setOverviewSlugLoading(false);
      return () => {
        cancelled = true;
      };
    }

    setResolvedOverviewTarget(null);
    setOverviewSlugLoading(true);
    const lookupLibrarySlug =
      parsedOverviewLibrarySlug === view ? null : parsedOverviewLibrarySlug;

    void resolveTitleOverviewTargetBySlug(
      backendClient,
      facet,
      lookupLibrarySlug,
      parsedOverviewSlug,
    )
      .then((target) => {
        if (cancelled) {
          return;
        }

        setResolvedOverviewTarget(target);
        if (!target) {
          navigateTo(view, undefined, "overview", undefined, undefined);
          return;
        }

        if (
          target.slug &&
          target.librarySlug &&
          (target.slug !== parsedOverviewSlug ||
            target.librarySlug !== parsedOverviewLibrarySlug)
        ) {
          navigateToOverview(view, target, overviewEpisodeId, true);
        }
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }

        setResolvedOverviewTarget(null);
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
        navigateTo(view, undefined, "overview", undefined, undefined);
      })
      .finally(() => {
        if (!cancelled) {
          setOverviewSlugLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [
    contentSettingsSection,
    navigateTo,
    navigateToOverview,
    navigationOverviewTarget,
    overviewEpisodeId,
    parsedOverviewLibrarySlug,
    parsedOverviewSlug,
    setGlobalStatus,
    t,
    view,
  ]);

  useEffect(() => {
    let cancelled = false;
    let keepPendingForNavigation = false;

    if (
      !routeIsCanonical ||
      !legacyOverviewTitleId ||
      !isMediaView(view) ||
      contentSettingsSection !== "overview"
    ) {
      setLegacyOverviewResolution(null);
      return () => {
        cancelled = true;
      };
    }

    const replaceWithOverviewList = () => {
      const nextParams = new URLSearchParams(searchParams.toString());
      nextParams.delete("id");
      nextParams.delete("episodeId");
      const nextQuery = nextParams.toString();
      navigate(`${buildViewPath(view)}${nextQuery ? `?${nextQuery}` : ""}`, {
        replace: true,
      });
    };

    setLegacyOverviewResolution(null);
    setOverviewSlugLoading(true);
    void resolveTitleOverviewTargetById(backendClient, legacyOverviewTitleId)
      .then((target) => {
        if (cancelled) {
          return;
        }
        if (!target) {
          replaceWithOverviewList();
          return;
        }

        const targetView = FACET_REGISTRY.find(
          (definition) => definition.id === target.facet,
        )?.viewId as ViewId | undefined;
        if (!targetView) {
          replaceWithOverviewList();
          return;
        }

        if (target.slug && target.librarySlug) {
          keepPendingForNavigation = true;
          navigateToOverview(targetView, target, overviewEpisodeId, true);
          return;
        }

        setLegacyOverviewResolution({
          requestedId: legacyOverviewTitleId,
          resolvedId: target.id,
        });
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }
        setGlobalStatus(
          error instanceof Error ? error.message : t("status.apiError"),
        );
        replaceWithOverviewList();
      })
      .finally(() => {
        if (!cancelled && !keepPendingForNavigation) {
          setOverviewSlugLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [
    contentSettingsSection,
    legacyOverviewTitleId,
    navigate,
    navigateToOverview,
    overviewEpisodeId,
    routeIsCanonical,
    searchParams,
    setGlobalStatus,
    t,
    view,
  ]);

  const overviewTitleId = parsedOverviewSlug
    ? (navigationOverviewTarget?.id ?? resolvedOverviewTarget?.id ?? null)
    : legacyOverviewResolution?.requestedId === legacyOverviewTitleId
      ? legacyOverviewResolution.resolvedId
      : null;
  const overviewTitleRoutePending = Boolean(
    isMediaView(view) &&
      overviewSlugLoading &&
      !overviewTitleId &&
      (parsedOverviewSlug || legacyOverviewTitleId),
  );

  const handleOpenOverview = useCallback(
    (
      targetView: ViewId,
      overviewTarget: OverviewTitleTarget,
      episodeId?: string,
    ) => {
      if (!isMediaView(targetView)) {
        return;
      }

      navigateToOverview(targetView, overviewTarget, episodeId);
    },
    [navigateToOverview],
  );

  const topNav = useMemo(
    () => [
      {
        id: "dashboard" as ViewId,
        label: t("nav.dashboard"),
        icon: LayoutDashboard,
      },
      ...FACET_REGISTRY.map((f) => ({
        id: f.viewId as ViewId,
        label: t(f.navLabelKey),
        icon: f.icon,
      })),
      {
        id: "discovery" as ViewId,
        label: t("nav.discovery"),
        icon: Sparkles,
      },
      {
        id: "activity" as ViewId,
        label: t("nav.activity"),
        icon: ActivitySquare,
      },
      {
        id: "calendar" as ViewId,
        label: t("nav.calendar"),
        icon: CalendarDays,
      },
      { id: "wanted" as ViewId, label: t("nav.wanted"), icon: ListChecks },
      { id: "system" as ViewId, label: t("system.title"), icon: Monitor },
      { id: "settings" as ViewId, label: t("nav.settings"), icon: Settings },
    ],
    [t],
  );
  const {
    canViewCatalog,
    canManageTitle,
    canRequestMedia,
    canResolveImports,
    canAccessActivity,
    canManageSystemSettings,
    canManageCatalogSettings,
    canManageUserAccounts,
    canManageUsers,
    canManageConfig,
    canManageLibrarySettings,
  } = usePermissions(authenticatedUser);
  const discoveryAuthorizationSignature = useMemo(
    () => authorizationCacheSignature(authenticatedUser),
    [authenticatedUser],
  );
  // Bulk actions run across a selection that can span libraries, so they need a
  // per-library answer rather than the "manages some library" aggregate.
  const canManageTitlesInLibrary = useCallback(
    (libraryId: string | null | undefined) =>
      hasLibraryPermission(
        authenticatedUser,
        libraryId,
        LIBRARY_PERMISSIONS.manageTitles,
      ),
    [authenticatedUser],
  );
  const {
    pendingImportCounts,
    pendingMediaRequestCounts,
    manualImportRequiredCount,
    pluginUpdateCount,
    scryerVersion,
  } = useNavigationBadges({
    serviceRestarting,
    canManageTitle,
    canRequestMedia,
  });
  const runningScryerVersion = scryerVersion?.trim() ?? "";
  const updateNoticeCurrentVersion =
    smgScryerUpdateNotice?.currentVersion.trim() ?? "";
  const updateNoticeMatchesRunningVersion =
    !runningScryerVersion ||
    !updateNoticeCurrentVersion ||
    runningScryerVersion === updateNoticeCurrentVersion;
  useEffect(() => {
    if (!updateNoticeMatchesRunningVersion) {
      void refreshSmgNotices({ force: true });
    }
  }, [refreshSmgNotices, updateNoticeMatchesRunningVersion]);
  const viewingBackupsSettings =
    view === "settings" && settingsSection === "backups";
  const globalSearchRouteCommands = useMemo(
    () =>
      buildRouteCommands({
        t,
        user: authenticatedUser,
        activityImportCount: manualImportRequiredCount,
        onNavigate: navigateTo,
      }),
    [authenticatedUser, manualImportRequiredCount, navigateTo, t],
  );

  useAutoBackupNotice({
    canManageSystemSettings,
    serviceRestarting,
    viewingBackupsSettings,
    navigateTo,
    t,
  });

  useEffect(() => {
    if (
      !routeIsCanonical ||
      !isMediaView(view) ||
      canAccessMediaSettingsSection(
        contentSettingsSection,
        canManageConfig,
        canManageLibrarySettings,
        canResolveImports,
      )
    ) {
      return;
    }

    navigateTo(
      view,
      undefined,
      fallbackMediaContentSettingsSection(
        contentSettingsSection,
        canManageConfig,
        canManageLibrarySettings,
        canResolveImports,
      ),
      undefined,
      undefined,
    );
  }, [
    canManageConfig,
    canManageLibrarySettings,
    canResolveImports,
    contentSettingsSection,
    navigateTo,
    routeIsCanonical,
    view,
  ]);

  const accessibleDefaultRoute = useMemo(
    () =>
      defaultAccessibleRoute(
        canViewCatalog,
        canRequestMedia,
        canResolveImports,
        canManageUserAccounts,
        canManageUsers,
        canManageSystemSettings,
        canManageCatalogSettings,
        canManageLibrarySettings,
      ),
    [
      canManageCatalogSettings,
      canManageLibrarySettings,
      canManageSystemSettings,
      canManageUserAccounts,
      canManageUsers,
      canResolveImports,
      canRequestMedia,
      canViewCatalog,
    ],
  );

  const navigateToAccessibleDefault = useCallback(() => {
    navigateTo(
      accessibleDefaultRoute.view,
      accessibleDefaultRoute.settingsSection,
      accessibleDefaultRoute.contentSettingsSection,
      undefined,
      undefined,
    );
  }, [accessibleDefaultRoute, navigateTo]);

  const routeCanAccessSettingsContent =
    view === "settings"
      ? canAccessSettingsSection(
          settingsSection,
          canManageUserAccounts,
          canManageUsers,
          canManageSystemSettings,
          canManageCatalogSettings,
        )
      : !(
          isMediaView(view) &&
          !canAccessMediaSettingsSection(
            contentSettingsSection,
            canManageConfig,
            canManageLibrarySettings,
            canResolveImports,
          )
        );
  const protectedSettingsRoute =
    routeCanAccessSettingsContent &&
    (isProtectedSettingsRoute(view, settingsSection, contentSettingsSection) ||
      (view === "system" &&
        systemSection === "recycleBin" &&
        canManageSystemSettings));
  const {
    refreshConfigStepUpPolicy,
    settingsStepUpCode,
    setSettingsStepUpCode,
    settingsStepUpBusy,
    settingsStepUpError,
    settingsStepUpOpen,
    settingsStepUpPolicyLoadFailed,
    settingsStepUpBlocksContent,
    handleCancelSettingsStepUp,
    handleSettingsStepUpSubmit,
  } = useConfigStepUp({
    authToken,
    initialMfaRequireConfigStepUp: mfaRequireConfigStepUp,
    protectedSettingsRoute,
    settingsSubscriptionEnabled: canSubscribeToLibraryEvents,
    adoptSession,
    setGlobalStatus,
    navigateTo,
    t,
  });

  // `/` has no path-derived destination: send the user to the best route their
  // permissions allow, which is the dashboard for operators and the catalog for
  // everyone else. Replaces rather than pushes so Back does not land on `/`
  // again and bounce forward.
  useEffect(() => {
    if (!routeIsLanding) {
      return;
    }

    // Carries the query and hash across, the way the old `/` redirect did, so
    // an entry link like `/?lang=fra` still reaches the landing page.
    navigate(
      `${buildViewPath(
        accessibleDefaultRoute.view,
        accessibleDefaultRoute.settingsSection,
        accessibleDefaultRoute.contentSettingsSection,
      )}${location.search}${location.hash}`,
      { replace: true },
    );
  }, [
    accessibleDefaultRoute,
    location.hash,
    location.search,
    navigate,
    routeIsLanding,
  ]);

  useEffect(() => {
    if (
      !routeIsCanonical ||
      view !== "dashboard" ||
      canAccessDashboard(canManageSystemSettings)
    ) {
      return;
    }

    navigateToAccessibleDefault();
  }, [
    canManageSystemSettings,
    navigateToAccessibleDefault,
    routeIsCanonical,
    view,
  ]);

  useEffect(() => {
    if (!routeIsCanonical || view !== "activity" || canAccessActivity) {
      return;
    }

    navigateToAccessibleDefault();
  }, [canAccessActivity, navigateToAccessibleDefault, routeIsCanonical, view]);

  useEffect(() => {
    if (
      !routeIsCanonical ||
      (view !== "calendar" && view !== "wanted") ||
      canViewCatalog
    ) {
      return;
    }

    navigateToAccessibleDefault();
  }, [canViewCatalog, navigateToAccessibleDefault, routeIsCanonical, view]);

  useEffect(() => {
    if (!routeIsCanonical || (view !== "system" && view !== "logs")) {
      return;
    }

    const canAccess =
      view === "logs"
        ? canManageSystemSettings
        : canAccessSystemSection(
            systemSection,
            canManageSystemSettings,
            canManageTitle,
          );
    if (canAccess) {
      return;
    }

    navigateToAccessibleDefault();
  }, [
    canManageTitle,
    canManageSystemSettings,
    navigateToAccessibleDefault,
    routeIsCanonical,
    systemSection,
    view,
  ]);

  useEffect(() => {
    if (
      !routeIsCanonical ||
      !isMediaView(view) ||
      contentSettingsSection !== "overview" ||
      canViewCatalog
    ) {
      return;
    }

    if (canRequestMedia) {
      navigateTo("requests");
      return;
    }

    if (canResolveImports) {
      navigateTo(view, undefined, "import", undefined, undefined);
      return;
    }

    if (canManageLibrarySettings && !canManageConfig) {
      navigateTo(view, undefined, "library", undefined, undefined);
      return;
    }

    navigateToAccessibleDefault();
  }, [
    canManageConfig,
    canManageLibrarySettings,
    canRequestMedia,
    canResolveImports,
    canViewCatalog,
    contentSettingsSection,
    navigateTo,
    navigateToAccessibleDefault,
    routeIsCanonical,
    view,
  ]);

  useEffect(() => {
    if (
      !routeIsCanonical ||
      view !== "requests" ||
      canManageTitle ||
      canRequestMedia
    ) {
      return;
    }

    navigateToAccessibleDefault();
  }, [
    canManageTitle,
    canRequestMedia,
    navigateToAccessibleDefault,
    routeIsCanonical,
    view,
  ]);

  useEffect(() => {
    if (!routeIsCanonical || view !== "settings") {
      return;
    }

    if (
      canAccessSettingsSection(
        settingsSection,
        canManageUserAccounts,
        canManageUsers,
        canManageSystemSettings,
        canManageCatalogSettings,
      )
    ) {
      return;
    }

    navigateTo(
      "settings",
      defaultSettingsSection(
        canManageSystemSettings,
        canManageCatalogSettings,
        canManageUserAccounts,
        canManageUsers,
      ),
    );
  }, [
    canManageCatalogSettings,
    canManageSystemSettings,
    canManageUserAccounts,
    canManageUsers,
    navigateTo,
    routeIsCanonical,
    settingsSection,
    view,
  ]);

  const handleBackToList = useCallback(() => {
    const targetPath = buildViewPath(view, undefined, "overview");
    const nextParams = new URLSearchParams(searchParams.toString());
    nextParams.delete(URL_PARAM_VIEW_DEPRECATED);
    nextParams.delete(URL_PARAM_SETTINGS_SECTION_DEPRECATED);
    nextParams.delete(URL_PARAM_CONTENT_SECTION_DEPRECATED);
    nextParams.delete(URL_PARAM_LANGUAGE);
    nextParams.delete("tab");
    nextParams.delete("id");
    nextParams.delete("episodeId");

    const nextQuery = nextParams.toString();
    const nextPathWithQuery = `${targetPath}${nextQuery ? `?${nextQuery}` : ""}`;
    navigate(nextPathWithQuery, {
      state: { restoreOverviewScroll: true },
    });
  }, [navigate, searchParams, view]);

  useEffect(() => {
    if (
      view !== "activity" ||
      activitySection !== "import" ||
      pendingImportCounts === null ||
      manualImportRequiredCount > 0
    ) {
      return;
    }

    navigateTo(
      "activity",
      undefined,
      undefined,
      undefined,
      undefined,
      "activity",
    );
  }, [
    activitySection,
    manualImportRequiredCount,
    navigateTo,
    pendingImportCounts,
    view,
  ]);

  return (
    <ScryerGraphqlProvider language={uiLanguage}>
      <TranslateContext.Provider value={t}>
        <GlobalStatusContext.Provider value={setGlobalStatus}>
          <div
            data-slot="root-app-frame"
            className="flex min-h-dvh flex-col overflow-x-hidden text-[var(--scry-body)]"
          >
            {serviceRestarting && <BackendRestartOverlay />}
            <Suspense fallback={<ViewLoadingFallback />}>
              <LibraryScanProgressProvider>
                <JobRunProvider enabled={canSubscribeToJobEvents}>
                  <ReactiveRefreshProvider
                    enabled={canSubscribeToLibraryEvents}
                  >
                    <GlobalSearchProvider
                      activeFacet={activeFacet}
                      authenticatedUser={authenticatedUser}
                      onOpenOverview={handleOpenOverview}
                      queueFacet={queueFacet}
                      uiLanguage={uiLanguage}
                    >
                      {smgVersionCompatibilityNotice ? (
                        <SmgUpgradeBanner
                          notice={smgVersionCompatibilityNotice}
                          t={t}
                        />
                      ) : null}

                      {showSmgScryerUpdateReminder &&
                      smgScryerUpdateNotice &&
                      updateNoticeMatchesRunningVersion ? (
                        <SmgScryerUpdateBanner
                          canManageSystemSettings={canManageSystemSettings}
                          notice={smgScryerUpdateNotice}
                          t={t}
                          onDismiss={dismissSmgScryerUpdateReminder}
                        />
                      ) : null}

                      {!isOnline ? (
                        <div
                          data-slot="root-shell-notice"
                          className="flex items-center justify-center gap-2 border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-surfA))] px-4 py-2 text-sm font-medium text-[var(--scry-body)] shadow-[0_8px_28px_rgba(2,6,23,0.14)] backdrop-blur"
                        >
                          <WifiOff className="h-4 w-4 flex-none text-[var(--scry-accent-ring)]" />
                          <span className="text-[var(--scry-ink2)]">
                            {t("pwa.offline")}
                          </span>
                        </div>
                      ) : null}

                      {showInstallBanner ? (
                        <div
                          data-slot="root-shell-notice"
                          className="flex items-center justify-center gap-3 border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,var(--scry-soft),var(--scry-surfA))] px-4 py-2 text-sm text-[var(--scry-body)] shadow-[0_8px_28px_rgba(var(--scry-accent-rgb),0.10)] backdrop-blur"
                        >
                          <Download className="h-4 w-4 flex-none text-[var(--scry-accent-ring)]" />
                          <span className="text-[var(--scry-muted)]">
                            {isIosSafari
                              ? t("pwa.iosInstallHint")
                              : t("pwa.installApp")}
                          </span>
                          {canPrompt ? (
                            <Button
                              type="button"
                              onClick={() => void promptInstall()}
                              variant="outline"
                              size="sm"
                              className="h-7 rounded-[8px] px-3 text-xs font-semibold text-[var(--scry-accent-text)]"
                            >
                              {t("pwa.installApp")}
                            </Button>
                          ) : null}
                          <IconButton
                            type="button"
                            onClick={dismissInstallBanner}
                            label={t("label.dismiss")}
                            appearance="ghost"
                            className="ml-auto h-7 w-7 rounded-[8px]"
                          >
                            <X className="h-4 w-4" />
                          </IconButton>
                        </div>
                      ) : null}

                      <Dialog
                        open={settingsStepUpOpen}
                        onOpenChange={(open) => {
                          if (!open && settingsStepUpOpen) {
                            handleCancelSettingsStepUp();
                          }
                        }}
                      >
                        <DialogContent
                          id="settings-mfa-step-up-dialog"
                          className="sm:max-w-md"
                          onInteractOutside={(event) => event.preventDefault()}
                        >
                          <DialogHeader>
                            <DialogTitle>
                              {t("settings.mfaStepUpTitle")}
                            </DialogTitle>
                            <DialogDescription>
                              {t("settings.mfaStepUpDescription")}
                            </DialogDescription>
                          </DialogHeader>
                          <TotpCodeForm
                            id="settings-mfa-step-up-form"
                            inputId="settings-mfa-step-up-code"
                            submitId="settings-mfa-step-up-submit"
                            cancelId="settings-mfa-step-up-cancel"
                            code={settingsStepUpCode}
                            title={t("auth.totpCode")}
                            description={t("auth.totpCodeRequired")}
                            submitLabel={t("settings.mfaStepUpSubmit")}
                            busyLabel={t("settings.mfaStepUpVerifying")}
                            cancelLabel={t("label.cancel")}
                            busy={settingsStepUpBusy}
                            onCodeChange={setSettingsStepUpCode}
                            onSubmit={handleSettingsStepUpSubmit}
                            onCancel={handleCancelSettingsStepUp}
                          />
                          {settingsStepUpError ? (
                            <p
                              id="settings-mfa-step-up-error"
                              className="text-sm text-destructive"
                            >
                              {settingsStepUpError}
                            </p>
                          ) : null}
                        </DialogContent>
                      </Dialog>

                      <div
                        ref={shellFrameRef}
                        data-slot="root-shell-frame"
                        className="flex min-h-0 w-full flex-1 min-[981px]:h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:max-h-[calc(100dvh-var(--root-shell-top-offset,0px))] min-[981px]:overflow-hidden"
                        style={
                          {
                            "--root-shell-top-offset": `${shellTopOffset}px`,
                          } as CSSProperties
                        }
                      >
                        <RootSidebar
                          topNav={topNav}
                          view={view}
                          settingsSection={settingsSection}
                          contentSettingsSection={contentSettingsSection}
                          systemSection={systemSection}
                          logsSection={logsSection}
                          activitySection={activitySection}
                          wantedSection={wantedSection}
                          user={authenticatedUser}
                          pendingImportCounts={pendingImportCounts}
                          pendingMediaRequestCounts={pendingMediaRequestCounts}
                          manualImportRequiredCount={manualImportRequiredCount}
                          pluginUpdateCount={pluginUpdateCount}
                          header={
                            <RootHeader
                              onOpenOverview={handleOpenOverview}
                              routeCommandItems={globalSearchRouteCommands}
                            />
                          }
                          onNavigate={navigateTo}
                        >
                          <main
                            data-slot="root-main-scroll"
                            className="flex min-h-[70vh] flex-1 flex-col min-[981px]:min-h-0 min-[981px]:overflow-y-auto"
                          >
                            <Suspense fallback={<ViewLoadingFallback />}>
                              {routeIsLanding ? (
                                <ViewLoadingFallback />
                              ) : !routeIsCanonical ? (
                                <NotFoundPage />
                              ) : settingsStepUpPolicyLoadFailed ? (
                                <div className="mx-auto flex min-h-[360px] w-full max-w-md flex-col items-center justify-center gap-3 px-6 py-12 text-center">
                                  <div className="flex h-12 w-12 items-center justify-center rounded-[14px] border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)] shadow-[0_12px_28px_rgba(2,6,23,0.12)]">
                                    <AlertTriangle
                                      className="h-6 w-6"
                                      aria-hidden="true"
                                    />
                                  </div>
                                  <h2 className="text-lg font-bold text-[var(--scry-ink2)]">
                                    {t("settings.mfaStepUpPolicyLoadFailed")}
                                  </h2>
                                  <p className="text-sm leading-6 text-[var(--scry-muted3)]">
                                    {t(
                                      "settings.mfaStepUpPolicyLoadFailedDescription",
                                    )}
                                  </p>
                                  <Button
                                    id="settings-mfa-step-up-policy-retry"
                                    type="button"
                                    onClick={() =>
                                      void refreshConfigStepUpPolicy()
                                    }
                                  >
                                    {t("settings.mfaStepUpPolicyRetry")}
                                  </Button>
                                </div>
                              ) : settingsStepUpBlocksContent ? (
                                <ViewLoadingFallback />
                              ) : (
                                <MainContent
                                  view={view}
                                  overviewTitleId={overviewTitleId}
                                  overviewTitleRoutePending={
                                    overviewTitleRoutePending
                                  }
                                  routeOverviewEpisodeId={overviewEpisodeId}
                                  handleBackToList={handleBackToList}
                                  settingsSection={settingsSection}
                                  userId={authenticatedUser.id}
                                  username={authenticatedUser.username}
                                  canManageTitlesInLibrary={canManageTitlesInLibrary}
                                  selectedLanguage={selectedLanguage}
                                  uiLanguage={uiLanguage}
                                  discoveryAuthorizationSignature={
                                    discoveryAuthorizationSignature
                                  }
                                  setLanguagePreferenceFromShell={
                                    setLanguagePreferenceFromShell
                                  }
                                  contentSettingsSection={
                                    contentSettingsSection
                                  }
                                  systemSection={systemSection}
                                  logsSection={logsSection}
                                  scryerVersion={scryerVersion}
                                  pluginUpdateCount={pluginUpdateCount}
                                  activitySection={activitySection}
                                  wantedSection={wantedSection}
                                  handleOpenOverview={handleOpenOverview}
                                  handleImportRouteEmpty={handleBackToList}
                                  canViewCatalog={canViewCatalog}
                                  canAccessActivity={canAccessActivity}
                                  canResolveImports={canResolveImports}
                                  canManageTitle={canManageTitle}
                                  canRequestMedia={canRequestMedia}
                                  canManageUserAccounts={canManageUserAccounts}
                                  canManageUsers={canManageUsers}
                                  canManageSystemSettings={
                                    canManageSystemSettings
                                  }
                                  canManageCatalogSettings={
                                    canManageCatalogSettings
                                  }
                                  canManageConfig={canManageConfig}
                                  canManageLibrarySettings={
                                    canManageLibrarySettings
                                  }
                                />
                              )}
                            </Suspense>
                          </main>
                        </RootSidebar>
                      </div>
                    </GlobalSearchProvider>
                  </ReactiveRefreshProvider>
                </JobRunProvider>
              </LibraryScanProgressProvider>
            </Suspense>
          </div>
        </GlobalStatusContext.Provider>
      </TranslateContext.Provider>
    </ScryerGraphqlProvider>
  );
}
