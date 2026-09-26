import * as React from "react";
import { Plus } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  ListAuthBadge,
  ListNoteTone,
  ListProviderItem,
  ListProviderManifest,
  ListProviderSettingChange,
  ListProviderSettings,
} from "@/lib/types/lists";
import { listIntervalParts, listKindLabelKey, publicProviders } from "@/lib/utils/lists";
import { cn } from "@/lib/utils";

import { ProviderSettingsCard } from "./provider-settings-card";
import { ProviderTile } from "./provider-tile";

const AUTH_BADGE_KEY: Record<ListAuthBadge, string> = {
  NO_ACCOUNT: "lists.catalog.auth.noAccount",
  NO_ACCOUNT_NEEDS_VALUE: "lists.catalog.auth.needsValue",
  MEMBER_ACCOUNT: "lists.catalog.auth.memberAccount",
  SERVER_API_KEY: "lists.catalog.auth.serverKey",
};

const NOTE_CLASS: Record<ListNoteTone, string> = {
  INFO: "border-[var(--scry-info-border)] bg-[var(--scry-info-bg)] text-[var(--scry-info-text)]",
  WARN: "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]",
  BAD: "border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] text-[var(--scry-danger-text)]",
};

type ProviderBrowserProps = {
  providers: ListProviderManifest[];
  onFollow: (manifest: ListProviderManifest, item: ListProviderItem) => void;
  /** Stored server-wide settings, with non-secret values; null until loaded. */
  providerSettings?: ListProviderSettings[] | null;
  onSaveProviderSettings?: (provider: string, changes: ListProviderSettingChange[]) => Promise<boolean>;
};

/** The catalog of public lists: a provider rail and that provider's followable lists. */
export function ProviderBrowser({ providers, onFollow, providerSettings, onSaveProviderSettings }: ProviderBrowserProps) {
  const t = useTranslate();
  const catalog = React.useMemo(() => publicProviders(providers), [providers]);
  const [selectedType, setSelectedType] = React.useState<string | null>(null);
  const selected = catalog.find((provider) => provider.providerType === selectedType) ?? catalog[0] ?? null;
  const settingFields = selected
    ? (providerSettings?.find((entry) => entry.providerType === selected.providerType)?.fields ?? selected.configFields)
    : [];

  if (!selected) {
    return (
      <p id="lists-catalog-empty" className="text-[13px] text-[var(--scry-muted)]">
        {t("lists.catalog.empty")}
      </p>
    );
  }

  return (
    <div id="lists-catalog" className="grid gap-4 md:grid-cols-[220px_minmax(0,1fr)]">
      <nav aria-label={t("lists.catalog.providers")} className="flex gap-1.5 overflow-x-auto md:flex-col md:overflow-visible">
        {catalog.map((provider) => {
          const active = provider.providerType === selected.providerType;
          return (
            <button
              key={provider.providerType}
              id={`lists-catalog-provider-${provider.providerType}`}
              type="button"
              aria-pressed={active}
              onClick={() => setSelectedType(provider.providerType)}
              className={cn(
                "flex flex-none items-center gap-2.5 rounded-[10px] border px-2.5 py-2 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--scry-focus)]",
                active
                  ? "border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.12)]"
                  : "border-transparent hover:bg-[var(--scry-hover)]",
              )}
            >
              <ProviderTile provider={provider} size="sm" />
              <span className="min-w-0">
                <span className="block truncate text-[13px] font-semibold text-[var(--scry-ink2)]">{provider.name}</span>
                {provider.summary ? (
                  <span className="hidden truncate text-[11.5px] text-[var(--scry-muted)] md:block">{provider.summary}</span>
                ) : null}
              </span>
            </button>
          );
        })}
      </nav>

      <div className="min-w-0 space-y-4">
        <div className="flex items-start gap-3">
          <ProviderTile provider={selected} />
          <div className="min-w-0">
            <h3 className="font-display text-[17px] font-bold text-[var(--scry-ink)]">{selected.name}</h3>
            {selected.blurb ? <p className="text-[13px] text-[var(--scry-muted)]">{selected.blurb}</p> : null}
          </div>
        </div>
        {onSaveProviderSettings && settingFields.length > 0 ? (
          <ProviderSettingsCard
            key={selected.providerType}
            providerType={selected.providerType}
            fields={settingFields}
            onSave={onSaveProviderSettings}
          />
        ) : null}
        {selected.notes.map((note) => (
          <p key={note.textKey} className={cn("rounded-[10px] border px-3 py-2 text-[12.5px]", NOTE_CLASS[note.tone])}>
            {t(note.textKey)}
          </p>
        ))}
        {selected.groups.map((group) => (
          <section key={group.label} className="space-y-2">
            <div className="flex items-center gap-2">
              <h4 className="text-[13px] font-semibold text-[var(--scry-ink2)]">{group.label}</h4>
              <Badge tone="outline" className="text-[10.5px]">
                {t(AUTH_BADGE_KEY[group.authBadge])}
              </Badge>
            </div>
            <ul className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
              {group.items.map((item) => {
                const interval = listIntervalParts(item.defaultIntervalSeconds);
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
                        onClick={() => onFollow(selected, item)}
                      >
                        <Plus className="h-3.5 w-3.5" />
                        {t("lists.catalog.follow")}
                      </Button>
                    </div>
                  </li>
                );
              })}
            </ul>
          </section>
        ))}
      </div>
    </div>
  );
}
