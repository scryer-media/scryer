import * as React from "react";
import { ListPlus, RefreshCw } from "lucide-react";

import { LoadingMark } from "@/components/common/loading-mark";
import { UnderlineFilterButton } from "@/components/common/underline-filter-button";
import type { ListsSection } from "@/components/root/types";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import type {
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
import type { AddListExclusionInput } from "@/lib/utils/lists";

import { AddByUrlCard } from "./add-by-url-card";
import { ExclusionsTable } from "./exclusions-table";
import { FollowListDialog, type FollowListTarget } from "./follow-list-dialog";
import { ListDetailPanel } from "./list-detail-panel";
import { ListTable } from "./list-table";
import { ProviderBrowser } from "./provider-browser";

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
  membershipOffset: number;
  error: string | null;
};

type ListsViewProps = {
  section: ListsSection;
  onSectionChange: (section: ListsSection) => void;
  canManageLists: boolean;
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
  membershipPageSize: number;
  onPreviewUrl: (url: string) => Promise<ListPreview | null>;
  onPreviewSource: (source: ListSourceDraft) => Promise<ListPreview | null>;
  onPreviewSubscription: (id: string) => Promise<ListPreview | null>;
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
        params: preview.params,
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

  return (
    <section id="lists-view" className="scry-scroll flex min-h-0 flex-1 overflow-y-auto bg-[var(--scry-surfE)]">
      <div className="mx-auto flex w-full max-w-[1240px] flex-col gap-4 px-4 py-6 sm:px-6 lg:px-8">
        <div className="flex items-start gap-4">
          <div className="flex h-11 w-11 flex-none items-center justify-center rounded-[13px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.16)] text-[var(--scry-accent-text)]">
            <ListPlus className="h-5 w-5" />
          </div>
          <div className="min-w-0 flex-1">
            <h1 className="font-display text-[25px] font-bold leading-tight text-[var(--scry-ink)]">
              {t("nav.lists")}
            </h1>
            <p className="mt-1 max-w-2xl text-[13.5px] text-[var(--scry-muted)]">{t("lists.heading.copy")}</p>
          </div>
          {canManageLists && section === "public" && subscriptions.length > 0 ? (
            <Button id="lists-sync-all" type="button" variant="outline" size="sm" onClick={props.onSyncAll}>
              <RefreshCw className="h-4 w-4" />
              {t("lists.action.syncAll")}
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
              count={subscriptions.length}
              onClick={() => onSectionChange("public")}
            />
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

        {loading ? (
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
            {canManageLists ? (
              <AddByUrlCard providers={providers} onPreviewUrl={props.onPreviewUrl} onRecognized={followFromUrl} />
            ) : null}

            <section className="space-y-3">
              <h2 className={PANEL_HEADING}>{t("lists.followed.heading")}</h2>
              {subscriptions.length === 0 ? (
                <p id="lists-empty" className="text-[13px] text-[var(--scry-muted)]">
                  {canManageLists ? t("lists.followed.emptyManager") : t("lists.followed.empty")}
                </p>
              ) : (
                <ListTable
                  subscriptions={subscriptions}
                  providers={providers}
                  canManageLists={canManageLists}
                  busyIds={busyIds}
                  onOpen={(subscription) => props.onOpenDetail(subscription.id)}
                  onSetEnabled={props.onSetEnabled}
                  onSyncNow={props.onSyncNow}
                />
              )}
            </section>

            {canManageLists ? (
              <section className="space-y-3">
                <h2 className={PANEL_HEADING}>{t("lists.catalog.heading")}</h2>
                <ProviderBrowser
                  providers={providers}
                  onFollow={followFromCatalog}
                  providerSettings={props.providerSettings}
                  onSaveProviderSettings={props.onSaveProviderSettings}
                />
              </section>
            ) : null}
          </div>
        )}
      </div>

      <ListDetailPanel
        detail={detail}
        fallback={detailFallback}
        provider={detailSubscription ? (providerByType.get(detailSubscription.source.provider) ?? null) : null}
        canManageLists={canManageLists}
        busy={detail ? busyIds.has(detail.id) : false}
        membershipPageSize={props.membershipPageSize}
        onClose={() => props.onOpenDetail(null)}
        onPage={props.onDetailPage}
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

      {canManageLists ? (
        <FollowListDialog
          target={followTarget}
          routeOptions={routeOptions}
          onClose={() => setFollowTarget(null)}
          onPreviewSource={props.onPreviewSource}
          onSubscribe={props.onSubscribe}
          onUpdate={props.onUpdate}
        />
      ) : null}
    </section>
  );
}
