import * as React from "react";
import { ListPlus, Puzzle, RefreshCw } from "lucide-react";

import { LoadingMark } from "@/components/common/loading-mark";
import { UnderlineFilterButton } from "@/components/common/underline-filter-button";
import type { ListsSection } from "@/components/root/types";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { FilteredPluginList } from "@/components/views/settings/filtered-plugin-list";
import { useTranslate } from "@/lib/context/translate-context";
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

type ListsViewProps = {
  section: ListsSection;
  onSectionChange: (section: ListsSection) => void;
  canManageLists: boolean;
  experimentalFeaturesEnabled: boolean;
  canManageProviderApps: boolean;
  onRefreshProviders: () => Promise<void>;
  accounts: ListAccount[];
  managedAccount: ListAccount | null;
  accountLoading: boolean;
  linkingProvider: string | null;
  accountLinkError: string | null;
  onLinkAccount: (provider: string) => void;
  onCancelLink: () => void;
  onManageAccount: (account: ListAccount | null) => void;
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
  onPreviewSubscription: (id: string, draft?: ListSubscriptionDraft) => Promise<ListPreview | null>;
  onSubscribe: (source: ListSourceDraft, draft: ListSubscriptionDraft) => Promise<boolean>;
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
  onSaveProviderSettings?: (provider: string, changes: ListProviderSettingChange[]) => Promise<boolean>;
};

const PANEL_HEADING = "font-display text-[17px] font-bold text-[var(--scry-ink)]";

export function ListsView(props: ListsViewProps) {
  const {
    section,
    onSectionChange,
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
  const [followTarget, setFollowTarget] = React.useState<FollowListTarget | null>(null);
  const canManageSubscriptions = canManageLists || (section === "personal" && props.experimentalFeaturesEnabled);
  const providerByType = React.useMemo(
    () => new Map(providers.map((provider) => [provider.providerType, provider])),
    [providers],
  );

  const followFromCatalog = (manifest: ListProviderManifest, item: ListProviderItem) => {
    setFollowTarget({
      kind: "new",
      source: { provider: manifest.providerType, sourceType: item.sourceType, params: [], url: null },
      manifest,
      item,
      name: item.name,
      kinds: item.kinds,
      preview: null,
    });
  };

  const followFromUrl = (url: string, preview: ListPreview) => {
    const manifest = preview.provider ? (providerByType.get(preview.provider) ?? null) : null;
    const item =
      manifest?.groups.flatMap((group) => group.items).find((entry) => entry.sourceType === preview.sourceType) ??
      null;
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

  const detailFallback = detail ? (subscriptions.find((entry) => entry.id === detail.id) ?? null) : null;
  const detailSubscription = detail?.subscription ?? detailFallback;
  const showPlugins = props.canManageProviderApps && (section === "public" || (section === "personal" && props.experimentalFeaturesEnabled));
  const contentRef = React.useRef<HTMLDivElement>(null);
  const [pluginsDocked, setPluginsDocked] = React.useState(false);
  const [pluginsOpen, setPluginsOpen] = React.useState(false);

  React.useEffect(() => {
    const content = contentRef.current;
    if (!content || !showPlugins) return;
    // Match the settings pages' minimum main width, plugin rail width and gap.
    const update = () => setPluginsDocked(content.clientWidth >= 1080 + 288 + 20);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(content);
    return () => observer.disconnect();
  }, [showPlugins]);

  React.useEffect(() => setPluginsOpen(false), [pluginsDocked, section]);

  const plugins = showPlugins ? (
    <FilteredPluginList
      family="LIST"
      title={t("lists.plugins.heading")}
      refreshProviderOptions={props.onRefreshProviders}
    />
  ) : null;

  return (
    <section id="lists-view" className="scry-scroll flex min-h-0 flex-1 overflow-y-auto bg-[var(--scry-surfE)]">
      <div className="w-full px-4 py-6 sm:px-6 lg:px-8">
      <div ref={contentRef} className="flex items-start justify-center gap-5">
      <div className="flex min-w-0 max-w-[1280px] flex-[1_1_1280px] flex-col gap-4">
        <div className="flex items-center gap-4">
          <div className="flex h-11 w-11 flex-none items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.16)] text-[var(--scry-accent-text)]">
            <ListPlus className="h-5 w-5" />
          </div>
          <div className="min-w-0 flex-1">
            <h1 className="font-display text-[25px] font-bold leading-tight text-[var(--scry-ink)]">
              {t("nav.lists")}
            </h1>
          </div>
          {showPlugins && !pluginsDocked ? (
            <Button type="button" variant="outline" size="sm" aria-expanded={pluginsOpen} aria-controls="lists-plugins-panel" onClick={() => setPluginsOpen(true)}>
              <Puzzle className="h-4 w-4" />
              {t("settings.plugins")}
            </Button>
          ) : null}
        </div>

        <div className="flex flex-wrap items-end gap-3 border-b border-[var(--scry-border3)]">
          <div role="tablist" aria-label={t("nav.lists")} className="relative top-px flex min-h-10 flex-wrap gap-x-5">
            <UnderlineFilterButton
              id="lists-tab-public"
              role="tab"
              aria-selected={section === "public"}
              selected={section === "public"}
              label={t("lists.tab.public")}
              count={section === "public" ? subscriptions.length : undefined}
              onClick={() => onSectionChange("public")}
            />
            {props.experimentalFeaturesEnabled ? (
              <UnderlineFilterButton id="lists-tab-personal" role="tab" aria-selected={section === "personal"} selected={section === "personal"} label={t("lists.tab.personal")} count={section === "personal" ? subscriptions.length : undefined} onClick={() => onSectionChange("personal")} />
            ) : null}
            {props.experimentalFeaturesEnabled && props.canManageProviderApps ? (
              <UnderlineFilterButton id="lists-tab-provider-apps" role="tab" aria-selected={section === "providerApps"} selected={section === "providerApps"} label={t("lists.providerApps.heading")} badge={t("label.advanced")} onClick={() => onSectionChange("providerApps")} />
            ) : null}
            {canManageLists ? (
              <UnderlineFilterButton
                id="lists-tab-exclusions"
                role="tab"
                aria-selected={section === "exclusions"}
                selected={section === "exclusions"}
                label={t("lists.tab.exclusions")}
                onClick={() => onSectionChange("exclusions")}
              />
            ) : null}
          </div>
        </div>

        {section === "providerApps" && props.canManageProviderApps && props.experimentalFeaturesEnabled ? (
          <ProviderAppsPanel />
        ) : (section === "providerApps" || (section === "personal" && !props.experimentalFeaturesEnabled)) ? (
          <p role="status" className="py-6 text-sm">{t("status.permissionDenied")}</p>
        ) : loading ? (
          <div className="flex items-center gap-2 py-8 text-sm text-[var(--scry-muted)]">
            <LoadingMark className="h-4 w-4" />
            {t("label.loading")}
          </div>
        ) : loadError ? (
          <div id="lists-load-error" className="space-y-2 py-6">
            <p className="text-sm text-[var(--scry-danger-text)]">{loadError}</p>
            <Button type="button" variant="outline" size="sm" onClick={onRetry}>
              {t("lists.action.retry")}
            </Button>
          </div>
        ) : section === "exclusions" && !canManageLists ? (
          <p id="lists-exclusions-not-permitted" role="status" className="py-6 text-sm text-muted-foreground">
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
            {section === "personal" ? (
              <PersonalAccounts providers={providers} accounts={props.accounts} subscriptions={subscriptions} managedAccount={props.managedAccount} accountLoading={props.accountLoading} busyIds={busyIds} linkingProvider={props.linkingProvider} linkError={props.accountLinkError} onLink={props.onLinkAccount} onCancelLink={props.onCancelLink} onManage={props.onManageAccount} onUnlink={props.onUnlinkAccount} onFollow={(manifest, item, source, name) => setFollowTarget({ kind: "new", source, manifest, item, name, kinds: item.kinds, preview: null })} />
            ) : null}

            <section className="space-y-3">
              <div className="flex min-h-8 items-center justify-between gap-3">
                <h2 className={PANEL_HEADING}>{t("lists.followed.heading")}</h2>
                {canManageSubscriptions && subscriptions.length > 0 ? (
                  <Button id="lists-sync-all" type="button" variant="primary" size="sm" onClick={props.onSyncAll}>
                    <RefreshCw className="h-4 w-4" />
                    {t("lists.action.syncAll")}
                  </Button>
                ) : null}
              </div>
              {subscriptions.length === 0 ? (
                <p id="lists-empty" className="text-[13px] text-[var(--scry-muted)]">
                  {canManageLists ? t("lists.followed.emptyManager") : t("lists.followed.empty")}
                </p>
              ) : (
                <ListTable
                  subscriptions={subscriptions}
                  providers={providers}
                  canManageLists={canManageSubscriptions}
                  busyIds={busyIds}
                  onOpen={(subscription) => props.onOpenDetail(subscription.id)}
                  onSetEnabled={props.onSetEnabled}
                  onSyncNow={props.onSyncNow}
                />
              )}
            </section>

            {canManageLists && section === "public" ? (
              <section className="space-y-3">
                <h2 className={PANEL_HEADING}>{t("lists.catalog.heading")}</h2>
                <ProviderBrowser
                  providers={providers}
                  subscriptions={subscriptions}
                  onFollow={followFromCatalog}
                  onPreviewUrl={props.onPreviewUrl}
                  onFollowUrl={followFromUrl}
                  providerSettings={props.providerSettings}
                  onSaveProviderSettings={props.onSaveProviderSettings}
                />
              </section>
            ) : null}
          </div>
        )}
      </div>
      {showPlugins && pluginsDocked ? (
        <aside aria-label={t("settings.plugins")} className="sticky top-[26px] min-w-[288px] max-w-[720px] flex-[1_1_720px]">
          {plugins}
        </aside>
      ) : null}
      </div>
      </div>
      {showPlugins && !pluginsDocked ? (
        <Sheet open={pluginsOpen} onOpenChange={setPluginsOpen}>
          <SheetContent id="lists-plugins-panel" side="right" className="w-[min(420px,calc(100vw-2rem))] overflow-y-auto">
            <SheetHeader><SheetTitle>{t("settings.plugins")}</SheetTitle></SheetHeader>
            <div className="px-3 pb-3">{plugins}</div>
          </SheetContent>
        </Sheet>
      ) : null}

      <ListDetailPanel
        detail={detail}
        fallback={detailFallback}
        provider={detailSubscription ? (providerByType.get(detailSubscription.source.provider) ?? null) : null}
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
