import * as React from "react";
import {
  ChevronRight,
  Edit,
  Logs,
  Lock,
  Plus,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { DownloadClientConfigField } from "@/components/common/download-client-config-field";
import {
  IndexerErrorHistoryModal,
  type IndexerErrorHistoryScope,
} from "@/components/common/indexer-error-history-modal";
import { PluginVisualLabel } from "@/components/common/plugin-visual";
import { ProxyAssignmentSelect } from "@/components/common/proxy-assignment-select";
import { IndexerCategoryPicker } from "@/components/views/media-content/indexer-category-picker";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { useTranslate } from "@/lib/context/translate-context";
import { visibleIndexerConfigFields } from "@/lib/types";
import type {
  IndexerRecord,
  IndexerDraft,
  ProxyRecord,
  ProviderTypeInfo,
  ConfigFieldDef,
  IndexerDownloadClientMappingCatalog,
  IndexerDownloadClientMappingCatalogResource,
  IndexerCategoryRoutingSettings,
  IndexerRoutingSettingsByScope,
} from "@/lib/types";
import type { ViewCategoryId } from "@/lib/types/quality-profiles";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  resolveConfigFieldsForValues,
  splitAdvancedConfigFields,
} from "@/lib/utils/provider-config-fields";
import { applyIndexerConfigOption } from "@/lib/utils/indexer-setup";
import { cn } from "@/lib/utils";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";
import {
  AUTOMATIC_DOWNLOAD_CLIENT_ID,
  getIndexerDownloadClientDraftMappingViewModel,
  getIndexerDownloadClientMappingViewModel,
  isManagementOnlyIndexer,
  type IndexerDownloadClientMappingViewModel,
} from "@/lib/utils/indexer-download-client-mapping";
import type { IndexerSettingsTab } from "@/components/root/types";
import type { SeedingProfileOption } from "@/lib/types/seeding-profiles";
import {
  SEEDING_PROFILE_INHERIT_VALUE,
  seedingProfileInheritOptionKey,
  seedingProfileSelectValue,
  seedingProfileSelectValueToId,
  supportsSeedingProfileAssignment,
} from "@/lib/utils/seeding-profiles";
import { LoadingMark } from "@/components/common/loading-mark";
import { getDefaultIndexerRouting } from "@/lib/constants/indexers";

type SettingsIndexersSectionProps = {
  /// Which pane of the Indexers page to render; the page's rail owns the choice.
  indexerSettingsTab?: IndexerSettingsTab;
  editingIndexerId: string | null;
  indexerDraft: IndexerDraft;
  setIndexerDraft: React.Dispatch<React.SetStateAction<IndexerDraft>>;
  submitIndexer: (
    event: React.FormEvent<HTMLFormElement>,
  ) => Promise<void> | void;
  mutatingIndexerId: string | null;
  resetIndexerDraft: () => void;
  settingsIndexerFilter: string;
  setSettingsIndexerFilter: (value: string) => void;
  settingsIndexers: IndexerRecord[];
  indexerDownloadClientMappingCatalogResource: IndexerDownloadClientMappingCatalogResource;
  refreshIndexerDownloadClientMappingCatalog: () => Promise<void> | void;
  mutatingIndexerMappingIds: ReadonlySet<string>;
  setIndexerDownloadClientMapping: (
    indexerId: string,
    downloadClientId: string | null,
  ) => Promise<void> | void;
  seedingProfileOptions: SeedingProfileOption[];
  mutatingIndexerSeedingProfileIds: ReadonlySet<string>;
  setIndexerSeedingProfile: (
    indexerId: string,
    seedingProfileId: string | null,
  ) => Promise<void> | void;
  mutatingIndexerProxyIds: ReadonlySet<string>;
  setIndexerProxyAssignment: (
    indexerId: string,
    proxyConfigId: string | null,
  ) => Promise<void> | void;
  proxyConfigs: ProxyRecord[];
  indexerRoutingByScope: IndexerRoutingSettingsByScope;
  indexerRoutingLoaded: boolean;
  indexerRoutingLoading: boolean;
  mutatingIndexerRoutingScopes: ReadonlySet<ViewCategoryId>;
  loadIndexerRouting: () => Promise<void> | void;
  updateIndexerRoutingForScope: (
    scope: ViewCategoryId,
    indexerId: string,
    nextValue: Partial<IndexerCategoryRoutingSettings>,
  ) => Promise<void> | void;
  editIndexer: (indexer: IndexerRecord) => void;
  updateIndexerToggles: (
    indexer: IndexerRecord,
    nextValue: Partial<
      Pick<
        IndexerRecord,
        "isEnabled" | "enableInteractiveSearch" | "enableAutoSearch"
      >
    >,
  ) => Promise<void> | void;
  deleteIndexer: (indexer: IndexerRecord) => Promise<void> | void;
  syncIndexer: (indexer: IndexerRecord) => Promise<void> | void;
  providerTypes: ProviderTypeInfo[];
  testIndexerConnection: () => Promise<void> | void;
  isTestingConnection: boolean;
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  startCreateIndexer: () => void;
};

const FALLBACK_PROVIDER_OPTIONS = [
  { value: "nzbgeek", label: "NZBGeek Indexer" },
  { value: "newznab", label: "Newznab Indexer" },
];

const INDEXER_NARROW_CELL_CLASS =
  "max-[1279px]:flex max-[1279px]:items-center max-[1279px]:justify-between max-[1279px]:gap-4 max-[1279px]:border-b max-[1279px]:border-border/60 max-[1279px]:px-3 max-[1279px]:py-2 max-[1279px]:text-right max-[1279px]:before:shrink-0 max-[1279px]:before:text-left max-[1279px]:before:text-xs max-[1279px]:before:font-medium max-[1279px]:before:text-muted-foreground max-[1279px]:before:content-[attr(data-label)]";

function selectedIndexerPresetName(
  fields: ConfigFieldDef[],
  key: string,
  value: string,
): string | null {
  const selectedOption = fields
    .find((field) => field.key === key)
    ?.options.find((option) => option.value === value);
  return selectedOption?.configOverrides?.some(
    (override) => override.key === "base_url",
  )
    ? selectedOption.label
    : null;
}

function formatIndexerProviderTypeLabel(
  providerType: string,
  t: ReturnType<typeof useTranslate>,
) {
  switch (providerType.trim().toLowerCase()) {
    case "usenet_indexer":
      return `Usenet ${t("settings.pluginCategoryIndexer")}`;
    case "torrent_indexer":
      return `Torrent ${t("settings.pluginCategoryIndexer")}`;
    default:
      return providerType;
  }
}

function IndexerProviderTypeCell({ providerType }: { providerType: string }) {
  const t = useTranslate();
  return (
    <PluginVisualLabel
      providerType={providerType}
      pluginType="indexer"
      label={formatIndexerProviderTypeLabel(providerType, t)}
      logoClassName="h-5 w-5 rounded-[6px]"
    />
  );
}

function IndexerActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof IconButton>, "tone"> & {
  label: string;
  tone: Extract<
    BoxedActionButtonTone,
    "edit" | "enabled" | "disabled" | "delete" | "search"
  >;
}) {
  return (
    <IconButton label={label} tone={tone} className={className} {...props}>
      {children}
    </IconButton>
  );
}

function formatRelativeTime(isoDate: string): string {
  const date = new Date(isoDate);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const absDiffMs = Math.abs(diffMs);
  const isFuture = diffMs < 0;

  const minutes = Math.floor(absDiffMs / 60_000);
  const hours = Math.floor(absDiffMs / 3_600_000);
  const days = Math.floor(absDiffMs / 86_400_000);

  let relative: string;
  if (minutes < 1) relative = "just now";
  else if (minutes < 60) relative = `${minutes}m ago`;
  else if (hours < 24) relative = `${hours}h ago`;
  else relative = `${days}d ago`;

  if (isFuture) {
    if (minutes < 60) relative = `in ${minutes}m`;
    else if (hours < 24) relative = `in ${hours}h`;
    else relative = `in ${days}d`;
  }

  return relative;
}

function IndexerStatusCell({
  indexer,
  onOpenErrorHistory,
}: {
  indexer: IndexerRecord;
  onOpenErrorHistory?: () => void;
}) {
  const t = useTranslate();
  if (!indexer.isEnabled) {
    return <span className="text-muted-foreground">{t("label.disabled")}</span>;
  }

  if (indexer.disabledUntil) {
    const until = new Date(indexer.disabledUntil);
    if (until > new Date()) {
      return (
        <span
          className="text-[var(--scry-warning-text)]"
          title={indexer.disabledUntil}
        >
          {t("settings.indexerDisabledUntil", {
            time: formatRelativeTime(indexer.disabledUntil),
          })}
        </span>
      );
    }
  }

  // Ahead of the last error: a cooling indexer is quiet on the indexer's own
  // instruction, and whatever error text is still on file predates that.
  if (
    indexer.rateLimitedUntil &&
    new Date(indexer.rateLimitedUntil) > new Date()
  ) {
    return (
      <span
        className="text-muted-foreground"
        title={t("settings.indexerCoolingDownHelp")}
      >
        {t("settings.indexerCoolingDownUntil", {
          time: formatRelativeTime(indexer.rateLimitedUntil),
        })}
      </span>
    );
  }

  if (indexer.lastErrorAt) {
    const content = t("settings.indexerLastError", {
      time: formatRelativeTime(indexer.lastErrorAt),
    });
    if (onOpenErrorHistory) {
      return (
        <button
          type="button"
          className="text-left text-[var(--scry-danger-text-soft)] underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          title={
            indexer.lastErrorMessage
              ? `${indexer.lastErrorMessage}\n${indexer.lastErrorAt}`
              : indexer.lastErrorAt
          }
          onClick={onOpenErrorHistory}
        >
          {content}
        </button>
      );
    }
    return (
      <span
        className="text-[var(--scry-danger-text-soft)]"
        title={
          indexer.lastErrorMessage
            ? `${indexer.lastErrorMessage}\n${indexer.lastErrorAt}`
            : indexer.lastErrorAt
        }
      >
        {content}
      </span>
    );
  }

  if (indexer.lastQueryAt) {
    return (
      <span className="text-muted-foreground" title={indexer.lastQueryAt}>
        {t("settings.indexerLastSearched", {
          time: formatRelativeTime(indexer.lastQueryAt),
        })}
      </span>
    );
  }

  return (
    <span className="text-muted-foreground">
      {t("settings.indexerNoActivity")}
    </span>
  );
}

/// The Indexers form renders the same declarations the download-client form
/// does, so it shares that renderer rather than keeping a second copy that
/// silently lags behind it — this one had no TAG or PATH branch, and would have
/// needed a third FILTERED_SELECT branch. Only the id prefix is pinned, because
/// selectors depend on it.
function DynamicConfigField(props: {
  field: ConfigFieldDef;
  value: string;
  hasStoredSecretValue?: boolean;
  onChange: (key: string, value: string) => void;
}) {
  return (
    <DownloadClientConfigField {...props} idPrefix="settings-indexer-field" />
  );
}

/// One provider's fields: everything that is not a checkbox in a grid, then the
/// checkboxes in a row under it. Used once for the standard fields and once
/// inside the advanced disclosure so the two groups lay out identically.
function ProviderConfigFieldGroup({
  fields,
  draft,
  onChange,
}: {
  fields: ConfigFieldDef[];
  draft: IndexerDraft;
  onChange: (key: string, value: string) => void;
}) {
  if (fields.length === 0) {
    return null;
  }
  const boolFields = fields.filter((field) => field.fieldType === "BOOL");
  const valueFor = (field: ConfigFieldDef, fallback: string) =>
    draft.configValues[field.key] ?? field.defaultValue ?? fallback;

  return (
    <div className="space-y-3">
      <div className="grid gap-3 md:grid-cols-3">
        {fields
          .filter((field) => field.fieldType !== "BOOL")
          .map((field) => (
            <DynamicConfigField
              key={field.key}
              field={field}
              value={valueFor(field, "")}
              hasStoredSecretValue={draft.storedSecretKeys.includes(field.key)}
              onChange={onChange}
            />
          ))}
      </div>
      {boolFields.length > 0 ? (
        <div className="flex items-center gap-6">
          {boolFields.map((field) => (
            <DynamicConfigField
              key={field.key}
              field={field}
              value={valueFor(field, "false")}
              hasStoredSecretValue={draft.storedSecretKeys.includes(field.key)}
              onChange={onChange}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}
function IndexerDownloadClientSelect({
  model,
  selectId,
  label,
  isPending,
  disabled = false,
  showLabel = false,
  compact = false,
  catalogError = null,
  onRetry,
  onChange,
}: {
  model: IndexerDownloadClientMappingViewModel;
  selectId: string;
  label: string;
  isPending: boolean;
  disabled?: boolean;
  showLabel?: boolean;
  compact?: boolean;
  catalogError?: string | null;
  onRetry?: () => Promise<void> | void;
  onChange: (downloadClientId: string | null) => Promise<void> | void;
}) {
  const t = useTranslate();
  const statusId = `${selectId}-status`;
  const selectedOption = model.options.find(
    (option) => option.id === model.selectedId,
  );
  const selectedLabel = model.isInvalid
    ? t("settings.indexerDownloadClientInvalidOption", {
        name: selectedOption?.name ?? model.selectedId,
      })
    : (selectedOption?.name ?? t("settings.indexerDownloadClientAutomatic"));

  if (model.isNotApplicable) {
    return (
      <div className="space-y-1.5">
        {showLabel ? <Label className="block">{label}</Label> : null}
        <span
          className="text-muted-foreground"
          data-testid={`${selectId}-not-applicable`}
        >
          {t("settings.indexerDownloadClientNotApplicable")}
        </span>
      </div>
    );
  }

  const invalidMessage = model.invalidReason
    ? t(
        `settings.indexerDownloadClientInvalid${
          model.invalidReason.charAt(0).toUpperCase() +
          model.invalidReason.slice(1)
        }`,
      )
    : null;

  return (
    <div className={compact ? "min-w-0" : "min-w-0 space-y-1.5"}>
      <Label className={showLabel ? "block" : "sr-only"} htmlFor={selectId}>
        {label}
      </Label>
      <Select
        value={model.selectedId}
        onValueChange={(value) =>
          void onChange(value === AUTOMATIC_DOWNLOAD_CLIENT_ID ? null : value)
        }
      >
        <SelectTrigger
          id={selectId}
          data-testid={selectId}
          className={compact ? "w-full max-w-48" : "w-full"}
          disabled={isPending || disabled}
          aria-describedby={
            model.isInvalid || model.isDisabled ? statusId : undefined
          }
          aria-busy={isPending}
        >
          <SelectValue>{selectedLabel}</SelectValue>
        </SelectTrigger>
        <SelectContent
          position="popper"
          className="w-80 max-w-[var(--radix-select-content-available-width)]"
        >
          <SelectItem value={AUTOMATIC_DOWNLOAD_CLIENT_ID}>
            {t("settings.indexerDownloadClientAutomatic")}
          </SelectItem>
          {model.options.map((option) => (
            <SelectItem key={option.id} value={option.id}>
              <span
                className={cn(
                  option.isCurrent &&
                    model.isInvalid &&
                    "text-[var(--scry-danger-text-soft)]",
                )}
              >
                {option.isCurrent && model.isInvalid
                  ? t("settings.indexerDownloadClientInvalidOption", {
                      name: option.name,
                    })
                  : option.name}
              </span>
              {!option.enabled ? (
                <span className="ml-1 text-xs text-[var(--scry-warning-text)]">
                  ({t("settings.indexerDownloadClientDisabled")})
                </span>
              ) : null}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {model.isInvalid ? (
        <div
          id={statusId}
          role="alert"
          className="flex flex-wrap items-center gap-1 text-xs text-[var(--scry-danger-text-soft)]"
        >
          <span>{invalidMessage}</span>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="h-auto px-1 py-0 text-xs"
            onClick={() => void onChange(null)}
            disabled={isPending || disabled}
          >
            {t("settings.indexerDownloadClientChooseAutomatic")}
          </Button>
        </div>
      ) : model.isDisabled ? (
        <p
          id={statusId}
          role="status"
          className="text-xs text-[var(--scry-warning-text)]"
        >
          {t("settings.indexerDownloadClientDisabledWarning", {
            name: model.currentClient?.name ?? selectedLabel,
          })}
        </p>
      ) : catalogError ? (
        <div className="flex flex-wrap items-center gap-1 text-xs text-[var(--scry-warning-text)]">
          <span>{t("settings.indexerDownloadClientCatalogStale")}</span>
          {onRetry ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-auto px-1 py-0 text-xs"
              onClick={() => void onRetry()}
            >
              {t("label.retry")}
            </Button>
          ) : null}
        </div>
      ) : isPending ? (
        <p
          id={statusId}
          role="status"
          className="text-xs text-muted-foreground"
        >
          {t("status.indexerDownloadClientMappingSaving")}
        </p>
      ) : null}
    </div>
  );
}

function IndexerDownloadClientCatalogPlaceholder({
  resource,
  selectId,
  label,
  showLabel = false,
  onRetry,
}: {
  resource: IndexerDownloadClientMappingCatalogResource;
  selectId: string;
  label: string;
  showLabel?: boolean;
  onRetry: () => Promise<void> | void;
}) {
  const t = useTranslate();
  const isLoading = resource.status === "idle" || resource.status === "loading";
  return (
    <div className="min-w-[210px] space-y-1.5">
      <Label className={showLabel ? "block" : "sr-only"} htmlFor={selectId}>
        {label}
      </Label>
      <Button
        id={selectId}
        type="button"
        variant="outline"
        className="w-full justify-start font-normal"
        disabled={isLoading}
        onClick={() => void onRetry()}
      >
        {isLoading
          ? t("settings.indexerDownloadClientLoading")
          : t("settings.indexerDownloadClientLoadRetry")}
      </Button>
      {!isLoading && resource.error ? (
        <p role="alert" className="text-xs text-[var(--scry-danger-text-soft)]">
          {resource.error}
        </p>
      ) : null}
    </div>
  );
}

function IndexerDownloadClientCell({
  indexer,
  resource,
  isPending,
  disabled,
  compact = false,
  onRetry,
  onChange,
}: {
  indexer: IndexerRecord;
  resource: IndexerDownloadClientMappingCatalogResource;
  isPending: boolean;
  disabled: boolean;
  compact?: boolean;
  onRetry: () => Promise<void> | void;
  onChange: (downloadClientId: string | null) => Promise<void> | void;
}) {
  const t = useTranslate();
  const selectId = selectorId("settings-indexer-download-client", indexer.id);
  const label = t("settings.indexerDownloadClientLabel", {
    name: indexer.name,
  });
  if (!resource.catalog) {
    return (
      <IndexerDownloadClientCatalogPlaceholder
        resource={resource}
        selectId={selectId}
        label={label}
        onRetry={onRetry}
      />
    );
  }
  return (
    <IndexerDownloadClientSelect
      model={getIndexerDownloadClientMappingViewModel(
        indexer,
        resource.catalog,
      )}
      selectId={selectId}
      label={label}
      isPending={isPending}
      disabled={disabled}
      compact={compact}
      catalogError={resource.status === "error" ? resource.error : null}
      onRetry={onRetry}
      onChange={onChange}
    />
  );
}

/**
 * Seeding-profile assignment for one indexer, rendered beside the
 * download-client mapping control. The backend rejects assignment on anything
 * that is not torrent-capable, so non-torrent indexers get the same
 * "not applicable" treatment the mapping control uses.
 */
function IndexerSeedingProfileSelect({
  selectId,
  label,
  value,
  options,
  supported,
  prowlarrManaged = false,
  prowlarrMinimumSeeders = null,
  isPending,
  disabled = false,
  showLabel = false,
  compact = false,
  onChange,
}: {
  selectId: string;
  label: string;
  value: string | null;
  options: SeedingProfileOption[];
  supported: boolean;
  /// Prowlarr supplied seed criteria for this child, so the null option means
  /// "use them" rather than "inherit the default".
  prowlarrManaged?: boolean;
  /// Prowlarr's imported `appMinimumSeeders` for this child, or null when it
  /// supplied none. It governs admission whether or not Prowlarr also sent
  /// goals, so the inherit option names it instead of claiming a bare default.
  prowlarrMinimumSeeders?: number | null;
  isPending: boolean;
  disabled?: boolean;
  showLabel?: boolean;
  compact?: boolean;
  onChange: (seedingProfileId: string | null) => Promise<void> | void;
}) {
  const t = useTranslate();
  const statusId = `${selectId}-status`;

  if (!supported) {
    return (
      <div className="space-y-1.5">
        {showLabel ? <Label className="block">{label}</Label> : null}
        <span
          className="text-muted-foreground"
          data-testid={`${selectId}-not-applicable`}
        >
          {t("settings.seedingProfileNotApplicable")}
        </span>
      </div>
    );
  }

  const isMissing =
    value !== null && !options.some((option) => option.id === value);
  const prowlarrMinimum = prowlarrMinimumSeeders ?? null;
  const inheritLabel = t(
    seedingProfileInheritOptionKey(prowlarrManaged, prowlarrMinimum),
    { count: prowlarrMinimum ?? 0 },
  );

  return (
    <div className={compact ? "min-w-0" : "min-w-0 space-y-1.5"}>
      <Label className={showLabel ? "block" : "sr-only"} htmlFor={selectId}>
        {label}
      </Label>
      <Select
        value={seedingProfileSelectValue(value)}
        onValueChange={(nextValue) =>
          void onChange(seedingProfileSelectValueToId(nextValue))
        }
      >
        <SelectTrigger
          id={selectId}
          data-testid={selectId}
          // Readable without opening the menu: the trigger only renders the
          // selected option's text, so the imported threshold needs its own
          // hook for assertions and for support reading a screenshot.
          data-prowlarr-minimum-seeders={
            prowlarrMinimum === null ? undefined : String(prowlarrMinimum)
          }
          className={compact ? "w-full max-w-48" : "w-full"}
          disabled={isPending || disabled}
          aria-describedby={isMissing ? statusId : undefined}
          aria-busy={isPending}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent
          position="popper"
          className="w-80 max-w-[var(--radix-select-content-available-width)]"
        >
          <SelectItem
            value={SEEDING_PROFILE_INHERIT_VALUE}
            data-testid={`${selectId}-inherit`}
          >
            {inheritLabel}
          </SelectItem>
          {isMissing && value ? (
            <SelectItem value={value}>
              {t("settings.seedingProfileMissing", { id: value })}
            </SelectItem>
          ) : null}
          {options.map((option) => (
            <SelectItem key={option.id} value={option.id}>
              {option.name}
            </SelectItem>
          ))}
          {options.length === 0 ? (
            // A Sonarr user lands here first; without this the dropdown is a
            // dead end that never says where profiles come from.
            <SelectItem value="__none-available" disabled>
              {t("settings.seedingProfileNoneAvailable")}
            </SelectItem>
          ) : null}
        </SelectContent>
      </Select>
      {isMissing ? (
        <p
          id={statusId}
          role="alert"
          className="text-xs text-[var(--scry-danger-text-soft)]"
        >
          {t("settings.seedingProfileMissing", { id: value })}
        </p>
      ) : isPending ? (
        <p
          id={statusId}
          role="status"
          className="text-xs text-muted-foreground"
        >
          {t("status.indexerSeedingProfileSaving")}
        </p>
      ) : null}
    </div>
  );
}

function IndexerSeedingProfileCell({
  indexer,
  catalog,
  options,
  isPending,
  disabled,
  compact = false,
  onChange,
}: {
  indexer: IndexerRecord;
  catalog: IndexerDownloadClientMappingCatalog | null;
  options: SeedingProfileOption[];
  isPending: boolean;
  disabled: boolean;
  compact?: boolean;
  onChange: (seedingProfileId: string | null) => Promise<void> | void;
}) {
  const t = useTranslate();
  const selectId = selectorId("settings-indexer-seeding-profile", indexer.id);
  if (!catalog) {
    return (
      <span
        className="text-muted-foreground"
        data-testid={`${selectId}-loading`}
      >
        {t("label.loading")}
      </span>
    );
  }
  const protocolFamilies = catalog.indexers.find(
    (entry) => entry.id === indexer.id,
  )?.protocolFamilies;
  return (
    <IndexerSeedingProfileSelect
      selectId={selectId}
      label={t("settings.seedingProfileIndexerLabel", { name: indexer.name })}
      value={indexer.seedingProfileId}
      options={options}
      prowlarrManaged={indexer.hasProwlarrSeedCriteria}
      prowlarrMinimumSeeders={indexer.prowlarrMinimumSeeders}
      supported={
        !isManagementOnlyIndexer(indexer) &&
        supportsSeedingProfileAssignment(protocolFamilies)
      }
      isPending={isPending}
      disabled={disabled}
      compact={compact}
      onChange={onChange}
    />
  );
}

const INDEXER_ROUTING_FACETS: Array<{
  scope: ViewCategoryId;
  labelKey: "search.facetMovie" | "search.facetSeries" | "search.facetAnime";
}> = [
  { scope: "MOVIE", labelKey: "search.facetMovie" },
  { scope: "SERIES", labelKey: "search.facetSeries" },
  { scope: "ANIME", labelKey: "search.facetAnime" },
];

function IndexerRoutingDisclosure({
  indexer,
  routingByScope,
  isLoading,
  mutatingScopes,
  onChange,
}: {
  indexer: IndexerRecord;
  routingByScope: IndexerRoutingSettingsByScope;
  isLoading: boolean;
  mutatingScopes: ReadonlySet<ViewCategoryId>;
  onChange: (
    scope: ViewCategoryId,
    indexerId: string,
    nextValue: Partial<IndexerCategoryRoutingSettings>,
  ) => Promise<void> | void;
}) {
  const t = useTranslate();

  return (
    <Table
      overflow="clip"
      layout="fixed"
      density="dense"
      wrapperClassName="rounded-[14px] border border-[var(--scry-border2)] bg-[var(--scry-surfC)]"
      className="[&_td]:align-middle [&_th]:align-middle"
    >
      <TableHeader>
        <TableRow>
          <TableHead className="w-32">{t("label.name")}</TableHead>
          <TableHead>{t("settings.indexerRoutingCategories")}</TableHead>
          <TableHead className="w-24 text-center">
            {t("settings.indexerRoutingEnabled")}
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {INDEXER_ROUTING_FACETS.map(({ scope, labelKey }) => {
          const routing =
            routingByScope[scope]?.[indexer.id] ??
            getDefaultIndexerRouting(scope);
          const isPending = isLoading || mutatingScopes.has(scope);
          const facetLabel = t(labelKey);
          const enabledId = selectorId(
            "settings-indexer-routing-enabled",
            indexer.id,
            scope,
          );

          return (
            <TableRow
              key={scope}
              data-ui="settings-table-row"
              data-subtable-row="indexer-routing"
            >
              <TableCell className="font-medium">{facetLabel}</TableCell>
              <TableCell>
                <div className="w-full max-w-md">
                  <IndexerCategoryPicker
                    triggerId={selectorId(
                      "settings-indexer-routing-categories",
                      indexer.id,
                      scope,
                    )}
                    panelId={selectorId(
                      "settings-indexer-routing-categories-panel",
                      indexer.id,
                      scope,
                    )}
                    categoryIdPrefix={selectorId(
                      "settings-indexer-routing-category",
                      indexer.id,
                      scope,
                    )}
                    value={routing.categories}
                    scope={scope}
                    capsCategories={indexer.capsCategories}
                    disabled={isPending}
                    categoriesLabel={`${t("settings.indexerRoutingCategories")} (${facetLabel})`}
                    onChange={(categories) =>
                      void onChange(scope, indexer.id, { categories })
                    }
                  />
                </div>
              </TableCell>
              <TableCell className="text-center">
                <Checkbox
                  id={enabledId}
                  size="large"
                  checked={routing.enabled}
                  disabled={isPending}
                  aria-label={`${t("settings.indexerRoutingEnabled")}: ${facetLabel}`}
                  onCheckedChange={(checked) =>
                    void onChange(scope, indexer.id, {
                      enabled: checked === true,
                    })
                  }
                />
              </TableCell>
            </TableRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

export function SettingsIndexersSection({
  indexerSettingsTab = "indexers",
  editingIndexerId,
  indexerDraft,
  setIndexerDraft,
  submitIndexer,
  mutatingIndexerId,
  resetIndexerDraft,
  settingsIndexerFilter,
  setSettingsIndexerFilter,
  settingsIndexers,
  indexerDownloadClientMappingCatalogResource,
  refreshIndexerDownloadClientMappingCatalog,
  mutatingIndexerMappingIds,
  setIndexerDownloadClientMapping,
  seedingProfileOptions,
  mutatingIndexerSeedingProfileIds,
  setIndexerSeedingProfile,
  mutatingIndexerProxyIds,
  setIndexerProxyAssignment,
  proxyConfigs,
  indexerRoutingByScope,
  indexerRoutingLoaded,
  indexerRoutingLoading,
  mutatingIndexerRoutingScopes,
  loadIndexerRouting,
  updateIndexerRoutingForScope,
  editIndexer,
  updateIndexerToggles,
  deleteIndexer,
  syncIndexer,
  providerTypes,
  testIndexerConnection,
  isTestingConnection,
  isEditorOpen,
  editorMode,
  startCreateIndexer,
}: SettingsIndexersSectionProps) {
  const t = useTranslate();
  const [errorHistoryIndexer, setErrorHistoryIndexer] =
    React.useState<IndexerErrorHistoryScope | null>(null);
  const [expandedRoutingIndexerIds, setExpandedRoutingIndexerIds] =
    React.useState<Set<string>>(() => new Set());
  const normalizedProviderType = indexerDraft.providerType.trim().toLowerCase();
  const isManagedSyncProvider = normalizedProviderType === "prowlarr";
  const isEditing = editorMode === "edit";
  const isSavingEditor = mutatingIndexerId === (editingIndexerId ?? "new");
  const showProxyColumn = proxyConfigs.length > 0;
  const toggleIndexerRouting = React.useCallback(
    (indexerId: string) => {
      const isOpening = !expandedRoutingIndexerIds.has(indexerId);
      setExpandedRoutingIndexerIds((previous) => {
        const next = new Set(previous);
        if (isOpening) {
          next.add(indexerId);
        } else {
          next.delete(indexerId);
        }
        return next;
      });
      if (isOpening) {
        void loadIndexerRouting();
      }
    },
    [expandedRoutingIndexerIds, loadIndexerRouting],
  );
  const indexersById = React.useMemo(() => {
    return new Map(settingsIndexers.map((indexer) => [indexer.id, indexer]));
  }, [settingsIndexers]);
  const managedChildCounts = React.useMemo(() => {
    const counts = new Map<string, number>();
    for (const indexer of settingsIndexers) {
      if (indexer.managedParentConfigId) {
        counts.set(
          indexer.managedParentConfigId,
          (counts.get(indexer.managedParentConfigId) ?? 0) + 1,
        );
      }
    }
    return counts;
  }, [settingsIndexers]);
  React.useEffect(() => {
    if (!isManagedSyncProvider || !indexerDraft.proxyConfigId) {
      return;
    }
    setIndexerDraft((previous) =>
      previous.proxyConfigId ? { ...previous, proxyConfigId: null } : previous,
    );
  }, [indexerDraft.proxyConfigId, isManagedSyncProvider, setIndexerDraft]);
  // Protocol families for the provider the editor is currently on: seeding
  // profiles only apply to torrent-capable indexers.
  const draftProtocolFamilies = React.useMemo(
    () =>
      indexerDownloadClientMappingCatalogResource.catalog?.providerCompatibility.find(
        (entry) =>
          entry.providerType.trim().toLowerCase() === normalizedProviderType,
      )?.protocolFamilies ?? [],
    [
      indexerDownloadClientMappingCatalogResource.catalog,
      normalizedProviderType,
    ],
  );

  // Build provider type options from loaded plugins, falling back to hardcoded list
  const providerTypeOptions = React.useMemo(() => {
    const baseOptions =
      providerTypes.length > 0
        ? providerTypes.map((pt) => ({
            value: pt.providerType,
            label: formatIndexerProviderTypeLabel(pt.name, t),
          }))
        : FALLBACK_PROVIDER_OPTIONS;

    if (!normalizedProviderType) {
      return baseOptions;
    }
    if (baseOptions.some((option) => option.value === normalizedProviderType)) {
      return baseOptions;
    }
    return [
      {
        value: normalizedProviderType,
        label: formatIndexerProviderTypeLabel(indexerDraft.providerType, t),
      },
      ...baseOptions,
    ];
  }, [indexerDraft.providerType, normalizedProviderType, providerTypes, t]);

  // Get config fields for the selected provider type
  const selectedProvider = React.useMemo(() => {
    return (
      providerTypes.find((pt) => pt.providerType === normalizedProviderType) ??
      null
    );
  }, [normalizedProviderType, providerTypes]);

  const selectedProviderFields = React.useMemo(
    () => visibleIndexerConfigFields(selectedProvider?.configFields ?? []),
    [selectedProvider],
  );

  // Conditions are resolved against the draft, so a field appears or becomes
  // required the moment the field it depends on changes.
  const { standard: standardProviderFields, advanced: advancedProviderFields } =
    React.useMemo(
      () =>
        splitAdvancedConfigFields(
          resolveConfigFieldsForValues(
            selectedProviderFields,
            indexerDraft.configValues,
          ),
        ),
      [indexerDraft.configValues, selectedProviderFields],
    );
  const [advancedConfigOpen, setAdvancedConfigOpen] = React.useState(false);
  const [hasCustomizedName, setHasCustomizedName] = React.useState(false);
  const wasEditorOpen = React.useRef(false);

  React.useEffect(() => {
    if (isEditorOpen && !wasEditorOpen.current) {
      const currentPresetName = selectedProviderFields
        .map((field) =>
          selectedIndexerPresetName(
            selectedProviderFields,
            field.key,
            indexerDraft.configValues[field.key] ?? field.defaultValue ?? "",
          ),
        )
        .find((name): name is string => name !== null);
      setHasCustomizedName(
        editorMode === "edit" && currentPresetName !== indexerDraft.name,
      );
    } else if (!isEditorOpen) {
      setHasCustomizedName(false);
    }
    wasEditorOpen.current = isEditorOpen;
  }, [
    editorMode,
    indexerDraft.configValues,
    indexerDraft.name,
    isEditorOpen,
    selectedProviderFields,
  ]);

  const handleConfigValueChange = React.useCallback(
    (key: string, value: string) => {
      const presetName = selectedIndexerPresetName(
        selectedProviderFields,
        key,
        value,
      );
      setIndexerDraft((prev) => ({
        ...prev,
        name:
          !hasCustomizedName && presetName !== null ? presetName : prev.name,
        configValues: applyIndexerConfigOption(
          selectedProviderFields,
          prev.configValues,
          key,
          value,
        ),
      }));
    },
    [hasCustomizedName, selectedProviderFields, setIndexerDraft],
  );

  const handleProviderTypeChange = React.useCallback(
    (nextProviderType: string) => {
      const nextProvider = providerTypes.find(
        (providerType) => providerType.providerType === nextProviderType,
      );
      const nextMappingCompatibility =
        indexerDownloadClientMappingCatalogResource.catalog?.providerCompatibility.find(
          (provider) => provider.providerType === nextProviderType,
        );
      setIndexerDraft((prev: IndexerDraft) => {
        const nextConfigValues: Record<string, string> = {};
        for (const field of nextProvider?.configFields ?? []) {
          if (field.valueSource === "HOST_BINDING") {
            continue;
          }
          nextConfigValues[field.key] =
            field.defaultValue ?? (field.fieldType === "BOOL" ? "false" : "");
        }
        return {
          ...prev,
          providerType: nextProviderType,
          downloadClientId:
            nextMappingCompatibility?.supportsMapping === false
              ? null
              : prev.downloadClientId,
          seedingProfileId: supportsSeedingProfileAssignment(
            nextMappingCompatibility?.protocolFamilies,
          )
            ? prev.seedingProfileId
            : null,
          storedSecretKeys: [],
          configValues: nextConfigValues,
        };
      });
    },
    [
      indexerDownloadClientMappingCatalogResource.catalog,
      providerTypes,
      setIndexerDraft,
    ],
  );

  const showIndexers = indexerSettingsTab === "indexers";

  return (
    <div id="settings-indexers-section" className="flex flex-col gap-4 text-sm">
      {showIndexers ? (
        <>
          <div
            id="settings-indexers-table-card"
            className="rounded border border-border"
          >
            <div className="flex items-center justify-between border-b border-border px-3 py-2">
              <CardTitle className="text-base">
                {t("settings.existingIndexers")}
              </CardTitle>
              <Input
                id="settings-indexers-filter"
                value={settingsIndexerFilter}
                onChange={(event) =>
                  setSettingsIndexerFilter(event.target.value)
                }
                placeholder={t("settings.indexerFilterPlaceholder")}
                className="max-w-64"
              />
            </div>
            <div className="min-w-0">
              <Table
                id="settings-indexers-table"
                overflow="clip"
                layout="fixed"
                density="dense"
                className="[&_td]:align-middle [&_td]:px-2 [&_th]:align-middle [&_th]:px-2 max-[1279px]:block max-[1279px]:[&_colgroup]:hidden max-[1279px]:[&_thead]:hidden max-[1279px]:[&_tbody]:block"
              >
                <colgroup>
                  <col className={showProxyColumn ? "w-[12%]" : "w-[15%]"} />
                  <col className={showProxyColumn ? "w-[9%]" : "w-[11%]"} />
                  {showProxyColumn ? <col className="w-[10%]" /> : null}
                  <col className="w-[14%]" />
                  <col className={showProxyColumn ? "w-[15%]" : "w-[20%]"} />
                  <col className="w-[5%]" />
                  <col className="w-[6%]" />
                  <col className="w-[4%]" />
                  <col className="w-[10%]" />
                  <col className="w-[15%]" />
                </colgroup>
                <TableHeader>
                  <TableRow>
                    <TableHead>{t("label.name")}</TableHead>
                    <TableHead>{t("settings.indexerProvider")}</TableHead>
                    {showProxyColumn ? (
                      <TableHead>{t("settings.proxyAssignment")}</TableHead>
                    ) : null}
                    <TableHead>{t("settings.indexerDownloadClient")}</TableHead>
                    <TableHead>{t("settings.seedingProfileColumn")}</TableHead>
                    <TableHead className="text-center">
                      {t("label.enabled")}
                    </TableHead>
                    <TableHead className="text-center">
                      {t("settings.indexerInteractiveSearch")}
                    </TableHead>
                    <TableHead className="text-center">
                      {t("settings.indexerAutoSearch")}
                    </TableHead>
                    <TableHead>{t("settings.indexerStatus")}</TableHead>
                    <TableHead className="whitespace-nowrap text-right">
                      {t("label.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {settingsIndexers.map((indexer) => {
                    const parentName = indexer.managedParentConfigId
                      ? indexersById.get(indexer.managedParentConfigId)?.name
                      : null;
                    const managedChildCount =
                      managedChildCounts.get(indexer.id) ?? 0;
                    const isRoutingExpanded = expandedRoutingIndexerIds.has(
                      indexer.id,
                    );
                    return (
                      <React.Fragment key={indexer.id}>
                        <TableRow
                          data-ui="settings-table-row"
                          id={selectorId("settings-indexer-row", indexer.name)}
                          className={cn(
                            indexer.isManaged && "bg-muted/25",
                            "cursor-pointer max-[1279px]:mb-3 max-[1279px]:block max-[1279px]:overflow-hidden max-[1279px]:rounded-lg max-[1279px]:border max-[1279px]:border-border",
                          )}
                          onClick={(event) => {
                            if (
                              event.target instanceof Element &&
                              event.target.closest(
                                "button, a, input, select, textarea, [role='button']",
                              )
                            ) {
                              return;
                            }
                            toggleIndexerRouting(indexer.id);
                          }}
                        >
                          <TableCell
                            data-label={t("label.name")}
                            className={INDEXER_NARROW_CELL_CLASS}
                          >
                            <div className="flex items-center gap-1">
                              <button
                                type="button"
                                className="inline-flex h-6 w-6 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
                                aria-label={t("settings.indexerRoutingScope", {
                                  scope: indexer.name,
                                })}
                                aria-controls={selectorId(
                                  "settings-indexer-routing",
                                  indexer.id,
                                )}
                                aria-expanded={isRoutingExpanded}
                                onClick={() => toggleIndexerRouting(indexer.id)}
                              >
                                <ChevronRight
                                  className={cn(
                                    "h-4 w-4 transition-transform",
                                    isRoutingExpanded && "rotate-90",
                                  )}
                                />
                              </button>
                              <div className="min-w-0 space-y-1">
                                <div className="font-medium">
                                  {indexer.name}
                                </div>
                                {indexer.isManaged ? (
                                  <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
                                    <span className="inline-flex items-center gap-1 rounded-full border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2 py-0.5 font-medium text-[var(--scry-warning-text)]">
                                      <Lock className="h-3 w-3" />
                                      {t("settings.managedIndexerBadge")}
                                    </span>
                                    <span>
                                      {parentName
                                        ? t("settings.managedByIndexer", {
                                            name: parentName,
                                          })
                                        : t("settings.managedByParent")}
                                    </span>
                                  </div>
                                ) : managedChildCount > 0 ? (
                                  <div className="text-xs text-muted-foreground">
                                    {t("settings.managesIndexerCount", {
                                      count: managedChildCount,
                                    })}
                                  </div>
                                ) : null}
                              </div>
                            </div>
                          </TableCell>
                          <TableCell
                            data-label={t("settings.indexerProvider")}
                            className={INDEXER_NARROW_CELL_CLASS}
                          >
                            <IndexerProviderTypeCell
                              providerType={indexer.providerType}
                            />
                          </TableCell>
                          {showProxyColumn ? (
                            <TableCell
                              data-label={t("settings.proxyAssignment")}
                              className={INDEXER_NARROW_CELL_CLASS}
                            >
                              <ProxyAssignmentSelect
                                selectId={selectorId(
                                  "settings-indexer-proxy",
                                  indexer.name,
                                )}
                                label={t("settings.proxyAssignment")}
                                proxies={proxyConfigs}
                                value={indexer.proxyConfigId ?? null}
                                disabled={
                                  indexer.isManaged ||
                                  mutatingIndexerProxyIds.has(indexer.id) ||
                                  (editingIndexerId === indexer.id &&
                                    isEditorOpen)
                                }
                                showLabel={false}
                                compact
                                onChange={(proxyConfigId) =>
                                  void setIndexerProxyAssignment(
                                    indexer.id,
                                    proxyConfigId,
                                  )
                                }
                              />
                            </TableCell>
                          ) : null}
                          <TableCell
                            data-label={t("settings.indexerDownloadClient")}
                            className={INDEXER_NARROW_CELL_CLASS}
                          >
                            <IndexerDownloadClientCell
                              indexer={indexer}
                              resource={
                                indexerDownloadClientMappingCatalogResource
                              }
                              isPending={mutatingIndexerMappingIds.has(
                                indexer.id,
                              )}
                              disabled={
                                editingIndexerId === indexer.id && isEditorOpen
                              }
                              compact
                              onRetry={
                                refreshIndexerDownloadClientMappingCatalog
                              }
                              onChange={(downloadClientId) =>
                                setIndexerDownloadClientMapping(
                                  indexer.id,
                                  downloadClientId,
                                )
                              }
                            />
                          </TableCell>
                          <TableCell
                            data-label={t("settings.seedingProfileColumn")}
                            className={INDEXER_NARROW_CELL_CLASS}
                          >
                            <IndexerSeedingProfileCell
                              indexer={indexer}
                              catalog={
                                indexerDownloadClientMappingCatalogResource.catalog
                              }
                              options={seedingProfileOptions}
                              isPending={mutatingIndexerSeedingProfileIds.has(
                                indexer.id,
                              )}
                              disabled={
                                editingIndexerId === indexer.id && isEditorOpen
                              }
                              compact
                              onChange={(seedingProfileId) =>
                                setIndexerSeedingProfile(
                                  indexer.id,
                                  seedingProfileId,
                                )
                              }
                            />
                          </TableCell>
                          <TableCell
                            data-label={t("label.enabled")}
                            className={cn(
                              "text-center",
                              INDEXER_NARROW_CELL_CLASS,
                            )}
                          >
                            <Checkbox
                              id={selectorId(
                                "settings-indexer-enabled",
                                indexer.id,
                              )}
                              size="large"
                              checked={indexer.isEnabled}
                              disabled={mutatingIndexerId === indexer.id}
                              aria-label={`${t("label.enabled")}: ${indexer.name}`}
                              onCheckedChange={(checked) =>
                                void updateIndexerToggles(indexer, {
                                  isEnabled: checked === true,
                                })
                              }
                            />
                          </TableCell>
                          <TableCell
                            data-label={t("settings.indexerInteractiveSearch")}
                            className={cn(
                              "text-center",
                              INDEXER_NARROW_CELL_CLASS,
                            )}
                          >
                            {indexer.supportsManagedChildrenSync ? (
                              <span
                                className="text-muted-foreground"
                                title={t("settings.indexerManagedParentHint")}
                              >
                                —
                              </span>
                            ) : (
                              <Checkbox
                                id={selectorId(
                                  "settings-indexer-interactive-search",
                                  indexer.id,
                                )}
                                size="large"
                                checked={indexer.enableInteractiveSearch}
                                disabled={mutatingIndexerId === indexer.id}
                                aria-label={`${t("settings.indexerInteractiveSearch")}: ${indexer.name}`}
                                onCheckedChange={(checked) =>
                                  void updateIndexerToggles(indexer, {
                                    enableInteractiveSearch: checked === true,
                                  })
                                }
                              />
                            )}
                          </TableCell>
                          <TableCell
                            data-label={t("settings.indexerAutoSearch")}
                            className={cn(
                              "text-center",
                              INDEXER_NARROW_CELL_CLASS,
                            )}
                          >
                            {indexer.supportsManagedChildrenSync ? (
                              <span
                                className="text-muted-foreground"
                                title={t("settings.indexerManagedParentHint")}
                              >
                                —
                              </span>
                            ) : (
                              <Checkbox
                                id={selectorId(
                                  "settings-indexer-auto-search",
                                  indexer.id,
                                )}
                                size="large"
                                checked={indexer.enableAutoSearch}
                                disabled={mutatingIndexerId === indexer.id}
                                aria-label={`${t("settings.indexerAutoSearch")}: ${indexer.name}`}
                                onCheckedChange={(checked) =>
                                  void updateIndexerToggles(indexer, {
                                    enableAutoSearch: checked === true,
                                  })
                                }
                              />
                            )}
                          </TableCell>
                          <TableCell
                            data-label={t("settings.indexerStatus")}
                            className={INDEXER_NARROW_CELL_CLASS}
                          >
                            <IndexerStatusCell
                              indexer={indexer}
                              onOpenErrorHistory={() =>
                                setErrorHistoryIndexer({
                                  id: indexer.id,
                                  name: indexer.name,
                                })
                              }
                            />
                          </TableCell>
                          <TableCell
                            data-label={t("label.actions")}
                            className={cn(
                              "text-right",
                              INDEXER_NARROW_CELL_CLASS,
                            )}
                          >
                            <div className="flex flex-nowrap items-center justify-end gap-2">
                              <IndexerActionButton
                                id={selectorId(
                                  "settings-indexer-error-history",
                                  indexer.name,
                                )}
                                tone="search"
                                onClick={() =>
                                  setErrorHistoryIndexer({
                                    id: indexer.id,
                                    name: indexer.name,
                                  })
                                }
                                label={t("indexerErrors.history")}
                              >
                                <Logs className="h-4 w-4" />
                              </IndexerActionButton>
                              {!indexer.isManaged &&
                              indexer.supportsManagedChildrenSync ? (
                                <IndexerActionButton
                                  id={selectorId(
                                    "settings-indexer-sync",
                                    indexer.name,
                                  )}
                                  tone="search"
                                  onClick={() => void syncIndexer(indexer)}
                                  disabled={mutatingIndexerId === indexer.id}
                                  label={t("settings.indexerSyncNow")}
                                >
                                  {mutatingIndexerId === indexer.id ? (
                                    <LoadingMark className="h-4 w-4" />
                                  ) : (
                                    <RefreshCw className="h-4 w-4" />
                                  )}
                                </IndexerActionButton>
                              ) : null}
                              {indexer.isManaged ? (
                                <span className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-1 text-xs text-muted-foreground">
                                  <Lock className="h-3 w-3" />
                                  {t("settings.managedIndexerBadge")}
                                </span>
                              ) : (
                                <>
                                  <IndexerActionButton
                                    id={selectorId(
                                      "settings-indexer-edit",
                                      indexer.name,
                                    )}
                                    tone="edit"
                                    onClick={() => editIndexer(indexer)}
                                    label={t("label.edit")}
                                  >
                                    <Edit className="h-4 w-4" />
                                  </IndexerActionButton>
                                  <IndexerActionButton
                                    id={selectorId(
                                      "settings-indexer-delete",
                                      indexer.name,
                                    )}
                                    tone="delete"
                                    onClick={() => void deleteIndexer(indexer)}
                                    disabled={mutatingIndexerId === indexer.id}
                                    label={
                                      mutatingIndexerId === indexer.id
                                        ? t("label.deleting")
                                        : t("label.delete")
                                    }
                                  >
                                    <Trash2 className="h-4 w-4" />
                                  </IndexerActionButton>
                                </>
                              )}
                            </div>
                          </TableCell>
                        </TableRow>
                        {isRoutingExpanded ? (
                          <TableRow
                            id={selectorId(
                              "settings-indexer-routing",
                              indexer.id,
                            )}
                            className="max-[1279px]:block"
                          >
                            <TableCell
                              colSpan={showProxyColumn ? 10 : 9}
                              className="bg-muted/20 p-3 max-[1279px]:block"
                            >
                              {indexerRoutingLoaded ? (
                                <div className="mx-auto w-full max-w-4xl">
                                  <IndexerRoutingDisclosure
                                    indexer={indexer}
                                    routingByScope={indexerRoutingByScope}
                                    isLoading={indexerRoutingLoading}
                                    mutatingScopes={mutatingIndexerRoutingScopes}
                                    onChange={updateIndexerRoutingForScope}
                                  />
                                </div>
                              ) : (
                                <div className="flex min-h-24 items-center justify-center">
                                  <LoadingMark className="h-5 w-5" />
                                </div>
                              )}
                            </TableCell>
                          </TableRow>
                        ) : null}
                      </React.Fragment>
                    );
                  })}
                  {settingsIndexers.length === 0 ? (
                    <TableRow id="settings-indexers-empty-row">
                      <TableCell
                        colSpan={showProxyColumn ? 10 : 9}
                        className="text-muted-foreground"
                      >
                        {t("settings.noIndexersFound")}
                      </TableCell>
                    </TableRow>
                  ) : null}
                </TableBody>
              </Table>
            </div>
          </div>

          {isEditorOpen ? (
            <>
              <div className="relative overflow-hidden rounded-xl">
                <Card aria-busy={isSavingEditor}>
                  <CardHeader className="flex items-center justify-between gap-3">
                    <CardTitle className="text-base">
                      {editingIndexerId
                        ? t("settings.indexerUpdate")
                        : t("settings.indexerCreate")}
                    </CardTitle>
                    <label className="flex shrink-0 items-center gap-3">
                      <Checkbox
                        id="settings-indexer-enabled"
                        size="large"
                        checked={indexerDraft.isEnabled}
                        disabled={mutatingIndexerId !== null}
                        onCheckedChange={(checked) =>
                          setIndexerDraft((prev) => ({
                            ...prev,
                            isEnabled: checked === true,
                          }))
                        }
                      />
                      <span className="text-sm font-medium">
                        {t("label.enabled")}
                      </span>
                    </label>
                  </CardHeader>
                  <CardContent>
                    <form
                      id="settings-indexer-form"
                      className="space-y-3"
                      onSubmit={submitIndexer}
                    >
                      <div className="grid gap-3 md:grid-cols-2">
                        <label>
                          <Label
                            className="mb-2 block"
                            htmlFor="settings-indexer-provider-type"
                          >
                            {t("form.providerTypePlaceholder")}
                          </Label>
                          <Select
                            value={normalizedProviderType || undefined}
                            onValueChange={handleProviderTypeChange}
                          >
                            <SelectTrigger
                              id="settings-indexer-provider-type"
                              className="w-full"
                            >
                              <SelectValue
                                placeholder={t("form.providerTypePlaceholder")}
                              >
                                {normalizedProviderType ? (
                                  <PluginVisualLabel
                                    providerType={normalizedProviderType}
                                    pluginType="indexer"
                                    label={
                                      providerTypeOptions.find(
                                        (option) =>
                                          option.value ===
                                          normalizedProviderType,
                                      )?.label ??
                                      formatIndexerProviderTypeLabel(
                                        normalizedProviderType,
                                        t,
                                      )
                                    }
                                  />
                                ) : null}
                              </SelectValue>
                            </SelectTrigger>
                            <SelectContent>
                              {providerTypeOptions.map((opt) => (
                                <SelectItem key={opt.value} value={opt.value}>
                                  <PluginVisualLabel
                                    providerType={opt.value}
                                    pluginType="indexer"
                                    label={opt.label}
                                  />
                                </SelectItem>
                              ))}
                            </SelectContent>
                          </Select>
                        </label>
                        <label>
                          <Label
                            className="mb-2 block"
                            htmlFor="settings-indexer-name"
                          >
                            {t("label.name")}
                          </Label>
                          <Input
                            id="settings-indexer-name"
                            value={indexerDraft.name}
                            onChange={(event) => {
                              setHasCustomizedName(true);
                              setIndexerDraft((prev: IndexerDraft) => ({
                                ...prev,
                                name: event.target.value,
                              }));
                            }}
                            required
                            placeholder={t("form.indexerNamePlaceholder")}
                          />
                        </label>
                        {!isManagedSyncProvider ? (
                          <label>
                            <Label
                              className="mb-2 block"
                              htmlFor="settings-indexer-max-queries-per-minute"
                            >
                              {t("form.indexerMaxQueriesPerMinute")}
                            </Label>
                            <Input
                              id="settings-indexer-max-queries-per-minute"
                              type="number"
                              inputMode="numeric"
                              min={1}
                              step={1}
                              value={indexerDraft.maxQueriesPerMinute}
                              onChange={(event) =>
                                setIndexerDraft((prev: IndexerDraft) => ({
                                  ...prev,
                                  maxQueriesPerMinute: event.target.value,
                                }))
                              }
                              placeholder={t(
                                "form.indexerMaxQueriesPerMinutePlaceholder",
                              )}
                            />
                          </label>
                        ) : null}
                      </div>

                      <div className="grid gap-3 md:grid-cols-2">
                        {!isManagedSyncProvider ? (
                          <ProxyAssignmentSelect
                            selectId="settings-indexer-proxy-select"
                            label={t("settings.proxyAssignment")}
                            proxies={proxyConfigs}
                            value={indexerDraft.proxyConfigId}
                            onChange={(proxyConfigId) =>
                              setIndexerDraft((prev: IndexerDraft) => ({
                                ...prev,
                                proxyConfigId,
                              }))
                            }
                          />
                        ) : null}
                        {indexerDownloadClientMappingCatalogResource.catalog ? (
                          <IndexerDownloadClientSelect
                            model={getIndexerDownloadClientDraftMappingViewModel(
                              normalizedProviderType,
                              indexerDraft.downloadClientId,
                              indexerDownloadClientMappingCatalogResource.catalog,
                            )}
                            selectId="settings-indexer-download-client-form"
                            label={t("settings.indexerDownloadClient")}
                            isPending={mutatingIndexerId !== null}
                            showLabel
                            catalogError={
                              indexerDownloadClientMappingCatalogResource.status ===
                              "error"
                                ? indexerDownloadClientMappingCatalogResource.error
                                : null
                            }
                            onRetry={refreshIndexerDownloadClientMappingCatalog}
                            onChange={(downloadClientId) =>
                              setIndexerDraft((previous) => ({
                                ...previous,
                                downloadClientId,
                              }))
                            }
                          />
                        ) : (
                          <IndexerDownloadClientCatalogPlaceholder
                            resource={
                              indexerDownloadClientMappingCatalogResource
                            }
                            selectId="settings-indexer-download-client-form"
                            label={t("settings.indexerDownloadClient")}
                            showLabel
                            onRetry={refreshIndexerDownloadClientMappingCatalog}
                          />
                        )}
                        {indexerDownloadClientMappingCatalogResource.catalog &&
                        !isManagedSyncProvider &&
                        supportsSeedingProfileAssignment(
                          draftProtocolFamilies,
                        ) ? (
                          <IndexerSeedingProfileSelect
                            selectId="settings-indexer-seeding-profile-form"
                            label={t("settings.seedingProfileColumn")}
                            value={indexerDraft.seedingProfileId}
                            options={seedingProfileOptions}
                            supported
                            isPending={mutatingIndexerId !== null}
                            showLabel
                            onChange={(seedingProfileId) =>
                              setIndexerDraft((previous) => ({
                                ...previous,
                                seedingProfileId,
                              }))
                            }
                          />
                        ) : null}
                      </div>

                      {selectedProviderFields.length > 0 ? (
                        <div className="space-y-3">
                          <Label className="text-sm font-medium">
                            {t("settings.indexerConfig")}
                          </Label>
                          <ProviderConfigFieldGroup
                            fields={standardProviderFields}
                            draft={indexerDraft}
                            onChange={handleConfigValueChange}
                          />
                          {advancedProviderFields.length > 0 ? (
                            <Collapsible
                              open={advancedConfigOpen}
                              onOpenChange={setAdvancedConfigOpen}
                            >
                              <CollapsibleTrigger asChild>
                                <button
                                  id="settings-indexer-advanced-toggle"
                                  type="button"
                                  className="flex items-center gap-1.5 rounded-[8px] py-1 text-sm font-medium text-[var(--scry-muted)] transition-colors hover:text-[var(--scry-ink2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                                >
                                  <ChevronRight
                                    className={cn(
                                      "h-4 w-4 transition-transform",
                                      advancedConfigOpen && "rotate-90",
                                    )}
                                  />
                                  {t("settings.advancedConfig")}
                                  <span className="text-[var(--scry-faint)]">
                                    ({advancedProviderFields.length})
                                  </span>
                                </button>
                              </CollapsibleTrigger>
                              <CollapsibleContent className="pt-3">
                                <ProviderConfigFieldGroup
                                  fields={advancedProviderFields}
                                  draft={indexerDraft}
                                  onChange={handleConfigValueChange}
                                />
                              </CollapsibleContent>
                            </Collapsible>
                          ) : null}
                        </div>
                      ) : null}

                      {isManagedSyncProvider ? (
                        <p className="text-sm text-muted-foreground">
                          {t("settings.indexerManagedParentHint")}
                        </p>
                      ) : null}
                      {!isManagedSyncProvider ? (
                        <div className="flex flex-wrap items-center gap-6">
                          <label className="flex items-center gap-3">
                            <Checkbox
                              id="settings-indexer-enable-interactive-search"
                              size="large"
                              checked={indexerDraft.enableInteractiveSearch}
                              disabled={mutatingIndexerId !== null}
                              onCheckedChange={(value) =>
                                setIndexerDraft((prev: IndexerDraft) => ({
                                  ...prev,
                                  enableInteractiveSearch: value === true,
                                }))
                              }
                            />
                            <span className="text-sm font-medium">
                              {t("settings.indexerInteractiveSearch")}
                            </span>
                          </label>
                          <label className="flex items-center gap-3">
                            <Checkbox
                              id="settings-indexer-enable-auto-search"
                              size="large"
                              checked={indexerDraft.enableAutoSearch}
                              disabled={mutatingIndexerId !== null}
                              onCheckedChange={(value) =>
                                setIndexerDraft((prev: IndexerDraft) => ({
                                  ...prev,
                                  enableAutoSearch: value === true,
                                }))
                              }
                            />
                            <span className="text-sm font-medium">
                              {t("settings.indexerAutoSearch")}
                            </span>
                          </label>
                        </div>
                      ) : null}
                      <div className="flex gap-2">
                        <Button
                          id="settings-indexer-save"
                          type="submit"
                          disabled={isSavingEditor}
                        >
                          {isSavingEditor
                            ? t("label.saving")
                            : editingIndexerId
                              ? t("settings.indexerUpdate")
                              : t("settings.indexerCreate")}
                        </Button>
                        <Button
                          id="settings-indexer-test-connection"
                          type="button"
                          variant="outline"
                          onClick={() => void testIndexerConnection()}
                          disabled={isTestingConnection}
                        >
                          {isTestingConnection
                            ? t("status.testingIndexerConnection")
                            : t("label.testConnection")}
                        </Button>
                        <Button
                          id="settings-indexer-cancel"
                          type="button"
                          variant="outline"
                          onClick={resetIndexerDraft}
                        >
                          {t("label.cancel")}
                        </Button>
                      </div>
                    </form>
                  </CardContent>
                </Card>
                {isSavingEditor ? (
                  <div
                    role="status"
                    aria-live="polite"
                    className="absolute inset-0 z-10 flex items-center justify-center bg-[rgba(3,7,18,0.72)] p-4 backdrop-blur-[1px]"
                  >
                    <div className="flex items-center gap-2 rounded-[10px] border border-[var(--scry-border2)] bg-[var(--scry-surf)] px-4 py-3 text-sm font-medium text-[var(--scry-ink2)] shadow-[0_12px_28px_rgba(2,6,23,0.28)]">
                      <LoadingMark className="h-4 w-4" />
                      {t("label.saving")}
                    </div>
                  </div>
                ) : null}
              </div>
              {isEditing ? (
                <div className="flex justify-center">
                  <AddNewButton
                    id="settings-indexer-create"
                    icon={Plus}
                    label={t("settings.indexerCreateNew")}
                    onClick={startCreateIndexer}
                    disabled={mutatingIndexerId !== null}
                  />
                </div>
              ) : null}
            </>
          ) : (
            <div className="flex justify-center">
              <AddNewButton
                id="settings-indexer-create"
                icon={Plus}
                label={t("settings.indexerCreateNew")}
                onClick={startCreateIndexer}
              />
            </div>
          )}
        </>
      ) : null}
      <IndexerErrorHistoryModal
        open={errorHistoryIndexer != null}
        onOpenChange={(open) => {
          if (!open) setErrorHistoryIndexer(null);
        }}
        indexer={errorHistoryIndexer}
      />
    </div>
  );
}
