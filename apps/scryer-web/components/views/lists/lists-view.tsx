import * as React from "react";
import { ListPlus, Puzzle, RefreshCw } from "lucide-react";

import { LoadingMark } from "@/components/common/loading-mark";
import { UnderlineFilterButton } from "@/components/common/underline-filter-button";
import type { ListsSection } from "@/components/root/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  GutterPanel,
  GutterPanelButton,
} from "@/components/common/gutter-panel";
import { FilteredPluginList } from "@/components/views/settings/filtered-plugin-list";
import { useTranslate } from "@/lib/context/translate-context";
import {
  type GutterPanelMemory,
  type GutterPanelSize,
  useGutterPanel,
} from "@/lib/hooks/use-gutter-panel";
import type { ListAccountMemberKey } from "@/lib/hooks/use-list-account-link";
import type {
  ListAccount,
  ListExclusion,
  ListMembershipPage,
  ListPreview,
  ListProviderItem,
  ListProviderManifest,
  ListProviderSettingChange,
  ListProviderSettings,
  ListSourceDraft,
  ListSubscription,
  ListSubscriptionDraft,
  ListSyncRun,
} from "@/lib/types/lists";
import type { LibraryRecord } from "@/lib/types/titles";
import { listParamInput, type AddListExclusionInput } from "@/lib/utils/lists";

import { ExclusionsTable } from "./exclusions-table";
import { FollowListDialog, type FollowListTarget } from "./follow-list-dialog";
import { ListDetailPanel } from "./list-detail-panel";
import { ListTable } from "./list-table";
import { ProviderBrowser } from "./provider-browser";
import { PersonalAccounts } from "./personal-accounts";
import { ProviderAppsPanel } from "./provider-apps-panel";

// The plugins panel is pinned to the right edge beside the centered lists and
// closes behind its edge tab before it would cover them.
const PLUGIN_PANEL_SIZE: GutterPanelSize = { width: 320, gap: 32 };

export type ListRouteOptions = {
  libraries: LibraryRecord[];
  qualityProfiles: Array<{ id: string; name: string }>;
};

export type ListDetailState = {
  id: string;
  loading: boolean;
  subscription: ListSubscription | null;
  memberships: ListMembershipPage | null;
  runs: ListSyncRun[];
  /** Older syncs exist beyond the ones loaded. */
  moreRuns: boolean;
  membershipOffset: number;
  error: string | null;
};

export type ListTabCounts = Partial<
  Record<Exclude<ListsSection, "providerApps">, number>
>;

type ListsViewProps = {
  section: ListsSection;
  onSectionChange: (section: ListsSection) => void;
  /** How many entries each section holds; a section not counted yet is absent. */
  tabCounts: ListTabCounts;
  canManageLists: boolean;
  canManageProviderApps: boolean;
  pluginsPanelMemory: GutterPanelMemory;
  onRefreshProviders: () => Promise<void>;
  accounts: ListAccount[];
  managedAccounts: ListAccount[];
  accountLoading: boolean;
  accountReadErrors: Readonly<Record<string, string>>;
  linkingProvider: string | null;
  checkingAccountKey: boolean;
  accountLinkError: string | null;
  onLinkAccount: (provider: string, memberKey?: ListAccountMemberKey) => void;
  onCancelLink: () => void;
  onManageAccounts: (accounts: ListAccount[]) => void;
  onUnlinkAccount: (account: ListAccount) => Promise<boolean>;
  loading: boolean;
  loadError: string | null;
  onRetry: () => void;
  providers: ListProviderManifest[];
  subscriptions: ListSubscription[];
  routeOptions: ListRouteOptions;
  busyIds: ReadonlySet<string>;
  detail: ListDetailState | null;
  onOpenDetail: (id: string | null) => void;
  onDetailPage: (id: string, offset: number) => void;
  onShowAllRuns: (id: string) => void;
  membershipPageSize: number;
  onPreviewUrl: (url: string) => Promise<ListPreview | null>;
  onPreviewSource: (source: ListSourceDraft) => Promise<ListPreview | null>;
  onPreviewSubscription: (
    id: string,
    draft?: ListSubscriptionDraft,
  ) => Promise<ListPreview | null>;
  onSubscribe: (
    source: ListSourceDraft,
    draft: ListSubscriptionDraft,
  ) => Promise<boolean>;
  onUpdate: (id: string, draft: ListSubscriptionDraft) => Promise<boolean>;
  onSetEnabled: (subscription: ListSubscription, enabled: boolean) => void;
  onSyncNow: (subscription: ListSubscription) => void;
  onSyncAll: () => void;
  onUnsubscribe: (subscription: ListSubscription) => Promise<boolean>;
  exclusions: ListExclusion[];
  exclusionsLoading: boolean;
  onAddExclusion: (input: AddListExclusionInput) => Promise<boolean>;
  onRemoveExclusion: (exclusion: ListExclusion) => Promise<boolean>;
  providerSettings?: ListProviderSettings[] | null;
  onSaveProviderSettings?: (
    provider: string,
    changes: ListProviderSettingChange[],
  ) => Promise<boolean>;
};

const PANEL_HEADING =
  "font-display text-[17px] font-bold text-[var(--scry-ink)]";

export function ListsView(props: ListsViewProps) {
  const {
    section,
    onSectionChange,
    tabCounts,
    canManageLists,
    loading,
    loadError,
    onRetry,
    providers,
    subscriptions,
    routeOptions,
    busyIds,
    detail,
  } = props;
  const t = useTranslate();
  const [followTarget, setFollowTarget] =
    React.useState<FollowListTarget | null>(null);
  const canManageSubscriptions = canManageLists || section === "personal";
  const providerByType = React.useMemo(
    () =>
      new Map(providers.map((provider) => [provider.providerType, provider])),
    [providers],
  );

  const followFromCatalog = (
    manifest: ListProviderManifest,
    item: ListProviderItem,
  ) => {
    setFollowTarget({
      kind: "new",
      source: {
        provider: manifest.providerType,
        sourceType: item.sourceType,
        params: [],
        url: null,
      },
      manifest,
      item,
      name: item.name,
      kinds: item.kinds,
      preview: null,
    });
  };

  const followFromUrl = (url: string, preview: ListPreview) => {
    const manifest = preview.provider
      ? (providerByType.get(preview.provider) ?? null)
      : null;
    const item =
      manifest?.groups
        .flatMap((group) => group.items)
        .find((entry) => entry.sourceType === preview.sourceType) ?? null;
    setFollowTarget({
      kind: "new",
      source: {
        provider: preview.provider ?? "",
        sourceType: preview.sourceType ?? "",
        params: preview.params.map(listParamInput),
        url,
      },
      manifest,
      item,
      name: preview.name ?? item?.name ?? "",
      kinds: preview.kinds.length > 0 ? preview.kinds : (item?.kinds ?? []),
      preview,
    });
  };

  const detailFallback = detail
    ? (subscriptions.find((entry) => entry.id === detail.id) ?? null)
    : null;
  const detailSubscription = detail?.subscription ?? detailFallback;
  // Every tab keeps the same rail so switching tabs never opens or closes it.
  const showPlugins = props.canManageProviderApps;
  const scrollRef = React.useRef<HTMLElement>(null);
  const contentRef = React.useRef<HTMLDivElement>(null);
  const pluginsPanel = useGutterPanel(
    scrollRef,
    contentRef,
    showPlugins ? PLUGIN_PANEL_SIZE : null,
    undefined,
    props.pluginsPanelMemory,
  );

  const plugins = showPlugins ? (
    <FilteredPluginList
      family="LIST"
      title={t("lists.plugins.heading")}
      refreshProviderOptions={props.onRefreshProviders}
    />
  ) : null;

  return (
    <section
      ref={scrollRef}
      id="lists-view"
      className="scry-scroll flex min-h-0 flex-1 overflow-y-auto bg-transparent [scrollbar-gutter:stable]"
    >
      <div className="w-full px-4 py-6 sm:px-6 lg:px-8">
        <div
          ref={contentRef}
          className="mx-auto flex max-w-[1280px] flex-col gap-4"
        >
          <div className="flex items-center gap-4">
            <div className="flex h-11 w-11 flex-none items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.16)] text-[var(--scry-accent-text)]">
              <ListPlus className="h-5 w-5" />
            </div>
            <div className="min-w-0 flex-1">
              <h1 className="flex flex-wrap items-center gap-2.5 font-display text-[25px] font-bold leading-tight text-[var(--scry-ink)]">
                {t("nav.lists")}
                <Badge id="lists-beta" tone="warning" className="text-[11px]">
                  {t("settings.pluginBeta")}
                </Badge>
              </h1>
            </div>
            {showPlugins ? (
              <GutterPanelButton
                panelId="lists-plugins-panel"
                label={t("settings.plugins")}
                icon={<Puzzle className="h-4 w-4" />}
                open={pluginsPanel.open}
                onOpenChange={pluginsPanel.setOpen}
              />
            ) : null}
          </div>

          <div className="flex flex-wrap items-end gap-3 border-b border-[var(--scry-border3)]">
            <div
              role="tablist"
              aria-label={t("nav.lists")}
              className="relative top-px flex min-h-12 flex-wrap gap-x-2"
            >
              <UnderlineFilterButton
                id="lists-tab-public"
                role="tab"
                size="lg"
                aria-selected={section === "public"}
                selected={section === "public"}
                label={t("lists.tab.public")}
                count={tabCounts.public}
                onClick={() => onSectionChange("public")}
              />
              <UnderlineFilterButton
                id="lists-tab-personal"
                role="tab"
                size="lg"
                aria-selected={section === "personal"}
                selected={section === "personal"}
                label={t("lists.tab.personal")}
                count={tabCounts.personal}
                onClick={() => onSectionChange("personal")}
              />
              {canManageLists ? (
                <UnderlineFilterButton
                  id="lists-tab-exclusions"
                  role="tab"
                  size="lg"
                  aria-selected={section === "exclusions"}
                  selected={section === "exclusions"}
                  label={t("lists.tab.exclusions")}
                  count={tabCounts.exclusions}
                  onClick={() => onSectionChange("exclusions")}
                />
              ) : null}
              {props.canManageProviderApps ? (
                <UnderlineFilterButton
                  id="lists-tab-provider-apps"
                  role="tab"
                  size="lg"
                  aria-selected={section === "providerApps"}
                  selected={section === "providerApps"}
                  label={t("lists.providerApps.heading")}
                  badge={t("label.advanced")}
                  onClick={() => onSectionChange("providerApps")}
                />
              ) : null}
            </div>
          </div>

          {section === "providerApps" && props.canManageProviderApps ? (
            <ProviderAppsPanel />
          ) : section === "providerApps" ? (
            <p role="status" className="py-6 text-sm">
              {t("status.permissionDenied")}
            </p>
          ) : loading ? (
            <div className="flex items-center gap-2 py-8 text-sm text-[var(--scry-muted)]">
              <LoadingMark className="h-4 w-4" />
              {t("label.loading")}
            </div>
          ) : loadError ? (
            <div id="lists-load-error" className="space-y-2 py-6">
              <p className="text-sm text-[var(--scry-danger-text)]">
                {loadError}
              </p>
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={onRetry}
              >
                {t("lists.action.retry")}
              </Button>
            </div>
          ) : section === "exclusions" && !canManageLists ? (
            <p
              id="lists-exclusions-not-permitted"
              role="status"
              className="py-6 text-sm text-muted-foreground"
            >
              {t("status.permissionDenied")}
            </p>
          ) : section === "exclusions" ? (
            <ExclusionsTable
              exclusions={props.exclusions}
              subscriptions={subscriptions}
              loading={props.exclusionsLoading}
              busyIds={busyIds}
              onAdd={props.onAddExclusion}
              onRemove={props.onRemoveExclusion}
            />
          ) : (
            <div className="space-y-8">
              <section className="space-y-3">
                <div className="flex min-h-8 items-center justify-between gap-3">
                  <h2 className={PANEL_HEADING}>
                    {t("lists.followed.heading")}
                  </h2>
                  {canManageSubscriptions && subscriptions.length > 0 ? (
                    <Button
                      id="lists-sync-all"
                      type="button"
                      variant="primary"
                      size="sm"
                      onClick={props.onSyncAll}
                    >
                      <RefreshCw className="h-4 w-4" />
                      {t("lists.action.syncAll")}
                    </Button>
                  ) : null}
                </div>
                {subscriptions.length === 0 ? (
                  <p
                    id="lists-empty"
                    className="text-[13px] text-[var(--scry-muted)]"
                  >
                    {canManageLists
                      ? t("lists.followed.emptyManager")
                      : t("lists.followed.empty")}
                  </p>
                ) : (
                  <ListTable
                    subscriptions={subscriptions}
                    providers={providers}
                    accounts={
                      section === "personal" ? props.accounts : undefined
                    }
                    canManageLists={canManageSubscriptions}
                    busyIds={busyIds}
                    onOpen={(subscription) =>
                      props.onOpenDetail(subscription.id)
                    }
                    onSetEnabled={props.onSetEnabled}
                    onSyncNow={props.onSyncNow}
                  />
                )}
              </section>

              {section === "personal" ? (
                <PersonalAccounts
                  providers={providers}
                  accounts={props.accounts}
                  subscriptions={subscriptions}
                  managedAccounts={props.managedAccounts}
                  accountLoading={props.accountLoading}
                  accountReadErrors={props.accountReadErrors}
                  busyIds={busyIds}
                  linkingProvider={props.linkingProvider}
                  checkingKey={props.checkingAccountKey}
                  linkError={props.accountLinkError}
                  onLink={props.onLinkAccount}
                  onCancelLink={props.onCancelLink}
                  onManage={props.onManageAccounts}
                  onUnlink={props.onUnlinkAccount}
                  onPreviewSource={props.onPreviewSource}
                  onFollow={(manifest, item, source, name) =>
                    setFollowTarget({
                      kind: "new",
                      source,
                      manifest,
                      item,
                      name,
                      kinds: item.kinds,
                      preview: null,
                    })
                  }
                />
              ) : null}

              {canManageLists && section === "public" ? (
                <section className="space-y-3">
                  <h2 className={PANEL_HEADING}>
                    {t("lists.catalog.heading")}
                  </h2>
                  <ProviderBrowser
                    providers={providers}
                    subscriptions={subscriptions}
                    onFollow={followFromCatalog}
                    onPreviewUrl={props.onPreviewUrl}
                    onFollowUrl={followFromUrl}
                    onPreviewSource={props.onPreviewSource}
                    providerSettings={props.providerSettings}
                    onSaveProviderSettings={props.onSaveProviderSettings}
                  />
                </section>
              ) : null}
            </div>
          )}
        </div>
      </div>
      {showPlugins ? (
        <GutterPanel
          id="lists-plugins-panel"
          label={t("settings.plugins")}
          frame={pluginsPanel.frame}
          open={pluginsPanel.open}
          onOpenChange={pluginsPanel.setOpen}
        >
          {plugins}
        </GutterPanel>
      ) : null}

      <ListDetailPanel
        detail={detail}
        fallback={detailFallback}
        provider={
          detailSubscription
            ? (providerByType.get(detailSubscription.source.provider) ?? null)
            : null
        }
        canManageLists={canManageSubscriptions}
        busy={detail ? busyIds.has(detail.id) : false}
        membershipPageSize={props.membershipPageSize}
        onClose={() => props.onOpenDetail(null)}
        onPage={props.onDetailPage}
        onShowAllRuns={props.onShowAllRuns}
        onPreview={props.onPreviewSubscription}
        onEdit={(subscription) =>
          setFollowTarget({
            kind: "edit",
            subscription,
            manifest: providerByType.get(subscription.source.provider) ?? null,
          })
        }
        onSetEnabled={props.onSetEnabled}
        onSyncNow={props.onSyncNow}
        onUnsubscribe={props.onUnsubscribe}
      />

      {canManageSubscriptions ? (
        <FollowListDialog
          target={followTarget}
          routeOptions={routeOptions}
          onClose={() => setFollowTarget(null)}
          onPreviewSource={props.onPreviewSource}
          onPreviewSubscription={props.onPreviewSubscription}
          onSubscribe={props.onSubscribe}
          onUpdate={props.onUpdate}
        />
      ) : null}
    </section>
  );
}
