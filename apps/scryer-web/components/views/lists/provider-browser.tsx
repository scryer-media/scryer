import * as React from "react";
import { Check, Link2, Plus } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
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
import { isListSourceFollowed, listIntervalParts, listKindLabelKey, publicProviders } from "@/lib/utils/lists";
import { cn } from "@/lib/utils";

import { AddByUrlCard } from "./add-by-url-card";
import { ProviderSettingsCard } from "./provider-settings-card";
import { ProviderTile } from "./provider-tile";

/** What a group of lists needs before it can be followed; nothing is said when it needs nothing. */
const AUTH_BADGE_KEY: Partial<Record<ListAuthBadge, string>> = {
  NO_ACCOUNT_NEEDS_VALUE: "lists.catalog.auth.needsValue",
  MEMBER_ACCOUNT: "lists.catalog.auth.memberAccount",
  SERVER_API_KEY: "lists.catalog.auth.serverKey",
};

/** The rail entry that follows a list by its address instead of from a provider's catalog. */
const CUSTOM = "__custom__";

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
 * lists, plus a custom entry that follows a list by its address.
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
  const selected =
    selectedType === CUSTOM
      ? null
      : (catalog.find((provider) => provider.providerType === selectedType) ?? catalog[0] ?? null);
  const settingFields = selected
    ? (providerSettings?.find((entry) => entry.providerType === selected.providerType)?.fields ?? selected.configFields)
    : [];

  return (
    <div id="lists-catalog" className="grid gap-4 md:grid-cols-[220px_minmax(0,1fr)]">
      {catalog.length === 0 ? (
        <p id="lists-catalog-empty" className="text-[13px] text-[var(--scry-muted)] md:col-span-2">
          {t("lists.catalog.empty")}
        </p>
      ) : null}
      <nav aria-label={t("lists.catalog.providers")} className="flex gap-1.5 overflow-x-auto md:flex-col md:overflow-visible">
        {catalog.map((provider) => {
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

      {selected ? (
        <div className="min-w-0 space-y-4">
          <div className="flex items-center gap-3">
            <ProviderTile provider={selected} />
            <h3 className="min-w-0 truncate font-display text-[17px] font-bold text-[var(--scry-ink)]">{selected.name}</h3>
          </div>
          {onSaveProviderSettings && settingFields.length > 0 ? (
            <ProviderSettingsCard
              key={selected.providerType}
              providerType={selected.providerType}
              fields={settingFields}
              onSave={onSaveProviderSettings}
            />
          ) : null}
          {selected.groups.map((group) => {
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
                <ul className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
                  {group.items.map((item) => {
                    const interval = listIntervalParts(item.defaultIntervalSeconds);
                    // Only a list that takes no value is one fixed source; the rest
                    // can be followed again with a different value.
                    const followed =
                      item.params.length === 0 &&
                      isListSourceFollowed(subscriptions, { provider: selected.providerType, sourceType: item.sourceType, params: [] });
                    return (
                      <li
                        key={item.id}
                        className="flex min-w-0 flex-col gap-2 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] p-3"
                      >
                        <div className="min-w-0 flex-1">
                          <p className="truncate text-[13.5px] font-semibold text-[var(--scry-ink)]">{item.name}</p>
                          {item.description ? (
                            <p className="line-clamp-2 text-[12px] text-[var(--scry-muted)]">{item.description}</p>
                          ) : null}
                          <div className="mt-1.5 flex flex-wrap gap-1">
                            {item.kinds.map((kind) => (
                              <Badge key={kind} tone="neutral" className="px-1.5 py-0 text-[10.5px]">
                                {t(listKindLabelKey(kind))}
                              </Badge>
                            ))}
                            {item.params.map((param) => (
                              <Badge key={param.key} tone="info" className="px-1.5 py-0 text-[10.5px]">
                                {param.label}
                              </Badge>
                            ))}
                          </div>
                        </div>
                        <div className="flex items-center justify-between gap-2">
                          <span className="text-[11.5px] text-[var(--scry-muted)]">
                            {t("lists.catalog.every", { interval: t(interval.key, { count: interval.count }) })}
                          </span>
                          <Button
                            id={`lists-catalog-follow-${selected.providerType}-${item.id}`}
                            type="button"
                            size="xs"
                            variant="outline"
                            disabled={followed}
                            onClick={() => onFollow(selected, item)}
                          >
                            {followed ? <Check className="h-3.5 w-3.5" /> : <Plus className="h-3.5 w-3.5" />}
                            {t(followed ? "lists.catalog.followed" : "lists.catalog.follow")}
                          </Button>
                        </div>
                      </li>
                    );
                  })}
                </ul>
              </section>
            );
          })}
        </div>
      ) : (
        <div className="min-w-0">
          <AddByUrlCard providers={providers} onPreviewUrl={onPreviewUrl} onRecognized={onFollowUrl} />
        </div>
      )}
    </div>
  );
}
