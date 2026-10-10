import * as React from "react";
import { Link2 } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  ListAuthBadge,
  ListPreview,
  ListProviderItem,
  ListProviderManifest,
  ListProviderSettingChange,
  ListProviderSettings,
  ListSubscription,
} from "@/lib/types/lists";
import { isListSourceFollowed, publicProviders } from "@/lib/utils/lists";
import { cn } from "@/lib/utils";

import { AddByUrlCard } from "./add-by-url-card";
import { CatalogListTable } from "./catalog-list-table";
import { ProviderSettingsCard } from "./provider-settings-card";
import { ProviderTile } from "./provider-tile";

/** What a group of lists needs before it can be followed; nothing is said when it needs nothing. */
const AUTH_BADGE_KEY: Partial<Record<ListAuthBadge, string>> = {
  MEMBER_ACCOUNT: "lists.catalog.auth.memberAccount",
  SERVER_API_KEY: "lists.catalog.auth.serverKey",
};

/** The rail entry for lists that come from an address the reader supplies. */
const CUSTOM = "__custom__";
/** The provider whose lists are feeds at any address; it has no rail entry of its own. */
const CUSTOM_PROVIDER_TYPE = "custom";

const RAIL_ENTRY =
  "flex flex-none items-center gap-2.5 rounded-[10px] border px-2.5 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]";
const RAIL_ENTRY_ACTIVE = "border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.12)]";
const RAIL_ENTRY_IDLE = "border-transparent hover:bg-[var(--scry-hover)]";

type ProviderBrowserProps = {
  providers: ListProviderManifest[];
  /** The followed public lists, so a list already followed is not offered again. */
  subscriptions: ListSubscription[];
  onFollow: (manifest: ListProviderManifest, item: ListProviderItem) => void;
  onPreviewUrl: (url: string) => Promise<ListPreview | null>;
  onFollowUrl: (url: string, preview: ListPreview) => void;
  /** Stored server-wide settings, with non-secret values; null until loaded. */
  providerSettings?: ListProviderSettings[] | null;
  onSaveProviderSettings?: (provider: string, changes: ListProviderSettingChange[]) => Promise<boolean>;
};

/**
 * The catalog of public lists: a provider rail and that provider's followable
 * lists, plus a custom entry that follows a list by its address and holds the
 * custom provider's lists.
 */
export function ProviderBrowser({
  providers,
  subscriptions,
  onFollow,
  onPreviewUrl,
  onFollowUrl,
  providerSettings,
  onSaveProviderSettings,
}: ProviderBrowserProps) {
  const t = useTranslate();
  const catalog = React.useMemo(() => publicProviders(providers), [providers]);
  const [selectedType, setSelectedType] = React.useState<string | null>(null);
  const rail = catalog.filter((provider) => provider.providerType !== CUSTOM_PROVIDER_TYPE);
  const selected =
    selectedType === CUSTOM
      ? null
      : (rail.find((provider) => provider.providerType === selectedType) ?? rail[0] ?? null);
  // The provider whose lists the pane offers: the custom entry offers the custom provider's.
  const shown = selected ?? catalog.find((provider) => provider.providerType === CUSTOM_PROVIDER_TYPE) ?? null;
  const settingFields = shown
    ? (providerSettings?.find((entry) => entry.providerType === shown.providerType)?.fields ?? shown.configFields)
    : [];

  return (
    <div id="lists-catalog" className="grid gap-4 md:grid-cols-[220px_minmax(0,1fr)]">
      {catalog.length === 0 ? (
        <p id="lists-catalog-empty" className="text-[13px] text-[var(--scry-muted)] md:col-span-2">
          {t("lists.catalog.empty")}
        </p>
      ) : null}
      <nav aria-label={t("lists.catalog.providers")} className="flex gap-1.5 overflow-x-auto md:flex-col md:overflow-visible">
        {rail.map((provider) => {
          const active = provider.providerType === selected?.providerType;
          return (
            <button
              key={provider.providerType}
              id={`lists-catalog-provider-${provider.providerType}`}
              type="button"
              aria-pressed={active}
              onClick={() => setSelectedType(provider.providerType)}
              className={cn(RAIL_ENTRY, active ? RAIL_ENTRY_ACTIVE : RAIL_ENTRY_IDLE)}
            >
              <ProviderTile provider={provider} size="sm" />
              <span className="min-w-0 truncate text-[13px] font-semibold text-[var(--scry-ink2)]">{provider.name}</span>
            </button>
          );
        })}
        <button
          id="lists-catalog-custom"
          type="button"
          aria-pressed={!selected}
          onClick={() => setSelectedType(CUSTOM)}
          className={cn(RAIL_ENTRY, selected ? RAIL_ENTRY_IDLE : RAIL_ENTRY_ACTIVE)}
        >
          <span
            aria-hidden="true"
            className="inline-flex h-8 w-8 flex-none items-center justify-center text-[var(--scry-ink2)]"
          >
            <Link2 className="h-5 w-5" />
          </span>
          <span className="min-w-0 truncate text-[13px] font-semibold text-[var(--scry-ink2)]">{t("label.custom")}</span>
        </button>
      </nav>

      <div className="min-w-0 space-y-4">
        {/* The rail already says which provider is open, so the pane starts with its lists. */}
        {selected ? null : (
          <AddByUrlCard providers={providers} onPreviewUrl={onPreviewUrl} onRecognized={onFollowUrl} />
        )}
        {shown && onSaveProviderSettings && settingFields.length > 0 ? (
          <ProviderSettingsCard
            key={shown.providerType}
            providerType={shown.providerType}
            fields={settingFields}
            onSave={onSaveProviderSettings}
          />
        ) : null}
        {shown?.groups.map((group) => {
          const authBadgeKey = AUTH_BADGE_KEY[group.authBadge];
          return (
            <section key={group.label} className="space-y-2">
              <div className="flex items-center gap-2">
                <h4 className="text-[13px] font-semibold text-[var(--scry-ink2)]">{group.label}</h4>
                {authBadgeKey ? (
                  <Badge tone="outline" className="text-[10.5px]">
                    {t(authBadgeKey)}
                  </Badge>
                ) : null}
              </div>
              <CatalogListTable
                rows={group.items.map((item) => ({
                  key: item.id,
                  name: item.name,
                  description: item.description,
                  kinds: item.kinds,
                  intervalSeconds: item.defaultIntervalSeconds,
                  // Only a list that takes no value is one fixed source; the rest
                  // can be followed again with a different value.
                  followed:
                    item.params.length === 0 &&
                    isListSourceFollowed(subscriptions, { provider: shown.providerType, sourceType: item.sourceType, params: [] }),
                  followId: `lists-catalog-follow-${shown.providerType}-${item.id}`,
                  onFollow: () => onFollow(shown, item),
                }))}
              />
            </section>
          );
        })}
      </div>
    </div>
  );
}
