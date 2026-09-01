import * as React from "react";
import {
  Edit,
  Logs,
  Lock,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Trash2,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import {
  IndexerErrorHistoryModal,
  type IndexerErrorHistoryScope,
} from "@/components/common/indexer-error-history-modal";
import { PluginVisualLabel } from "@/components/common/plugin-visual";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox, CheckboxField } from "@/components/ui/checkbox";
import { Input, signedIntegerInputProps } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
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
  IndexerProxyDraft,
  IndexerProxyRecord,
  ProviderTypeInfo,
  ConfigFieldDef,
  IndexerDownloadClientMappingCatalog,
  IndexerDownloadClientMappingCatalogResource,
} from "@/lib/types";
import { selectorId } from "@/lib/utils/dom-ids";
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
  indexerProxyConfigs: IndexerProxyRecord[];
  indexerProxyDraft: IndexerProxyDraft;
  setIndexerProxyDraft: React.Dispatch<React.SetStateAction<IndexerProxyDraft>>;
  editingProxyId: string | null;
  isProxyEditorOpen: boolean;
  mutatingProxyId: string | null;
  testingProxyId: string | null;
  submitIndexerProxy: (
    event: React.FormEvent<HTMLFormElement>,
  ) => Promise<void> | void;
  resetIndexerProxyDraft: () => void;
  startCreateIndexerProxy: () => void;
  editIndexerProxy: (proxy: IndexerProxyRecord) => void;
  testIndexerProxy: (proxy: IndexerProxyRecord) => Promise<void> | void;
  deleteIndexerProxy: (proxy: IndexerProxyRecord) => Promise<void> | void;
  editIndexer: (indexer: IndexerRecord) => void;
  toggleIndexerEnabled: (indexer: IndexerRecord) => Promise<void> | void;
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

function formatIndexerProxyHealth(status: string | null | undefined): string {
  if (!status) return "Unknown";
  const normalized = status.toLowerCase();
  if (normalized === "healthy") return "Healthy";
  if (normalized === "unhealthy") return "Unhealthy";
  return "Unknown";
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

function DynamicConfigField({
  field,
  value,
  hasStoredSecretValue = false,
  onChange,
}: {
  field: ConfigFieldDef;
  value: string;
  hasStoredSecretValue?: boolean;
  onChange: (key: string, value: string) => void;
}) {
  const t = useTranslate();
  const fieldId = selectorId("settings-indexer-field", field.key);
  const requiredMarker = field.required ? (
    <span aria-hidden="true" className="text-destructive">
      *
    </span>
  ) : null;

  if (field.fieldType === "BOOL") {
    return (
      <CheckboxField
        id={fieldId}
        checked={value === "true"}
        onCheckedChange={(checkedValue) =>
          onChange(field.key, checkedValue === true ? "true" : "false")
        }
        label={field.label}
        labelAccessory={requiredMarker}
        description={field.helpText}
        className="items-center"
        checkboxClassName="mt-0"
      />
    );
  }

  if (field.fieldType === "SELECT" && field.options.length > 0) {
    return (
      <label>
        <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
        </Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(v) => onChange(field.key, v)}
        >
          <SelectTrigger id={fieldId} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((opt) => (
              <SelectItem key={opt.value} value={opt.value}>
                {opt.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {field.helpText ? (
          <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  if (field.fieldType === "MULTILINE") {
    return (
      <label>
        <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
          {field.label}
          {requiredMarker}
        </Label>
        <Textarea
          id={fieldId}
          value={value}
          onChange={(e) => onChange(field.key, e.target.value)}
          required={field.required && !hasStoredSecretValue}
          placeholder={field.defaultValue ?? ""}
          rows={6}
        />
        {field.helpText ? (
          <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  return (
    <label>
      <Label className="mb-2 inline-flex items-center gap-2" htmlFor={fieldId}>
        {field.label}
        {requiredMarker}
      </Label>
      <Input
        id={fieldId}
        value={value}
        onChange={(e) => onChange(field.key, e.target.value)}
        {...(field.fieldType === "NUMBER" ? signedIntegerInputProps : {})}
        type={
          field.fieldType === "PASSWORD"
            ? "password"
            : field.fieldType === "NUMBER"
              ? "number"
              : "text"
        }
        required={field.required && !hasStoredSecretValue}
        placeholder={
          hasStoredSecretValue
            ? t("form.apiKeyStoredPlaceholder")
            : field.defaultValue ?? ""
        }
      />
      {field.helpText ? (
        <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
      ) : null}
    </label>
  );
}

function IndexerDownloadClientSelect({
  model,
  selectId,
  label,
  isPending,
  disabled = false,
  showLabel = false,
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
    : selectedOption?.name ?? t("settings.indexerDownloadClientAutomatic");

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
    ? t(`settings.indexerDownloadClientInvalid${
        model.invalidReason.charAt(0).toUpperCase() + model.invalidReason.slice(1)
      }`)
    : null;

  return (
    <div className="min-w-0 space-y-1.5">
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
          className="w-full"
          disabled={isPending || disabled}
          aria-describedby={model.isInvalid || model.isDisabled ? statusId : undefined}
          aria-busy={isPending}
        >
          <SelectValue>{selectedLabel}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={AUTOMATIC_DOWNLOAD_CLIENT_ID}>
            {t("settings.indexerDownloadClientAutomatic")}
          </SelectItem>
          {model.options.map((option) => (
            <SelectItem key={option.id} value={option.id}>
              <span className={cn(option.isCurrent && model.isInvalid && "text-[var(--scry-danger-text-soft)]")}>
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
        <p id={statusId} role="status" className="text-xs text-muted-foreground">
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
  onRetry,
  onChange,
}: {
  indexer: IndexerRecord;
  resource: IndexerDownloadClientMappingCatalogResource;
  isPending: boolean;
  disabled: boolean;
  onRetry: () => Promise<void> | void;
  onChange: (downloadClientId: string | null) => Promise<void> | void;
}) {
  const t = useTranslate();
  const selectId = selectorId("settings-indexer-download-client", indexer.id);
  const label = t("settings.indexerDownloadClientLabel", { name: indexer.name });
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
      model={getIndexerDownloadClientMappingViewModel(indexer, resource.catalog)}
      selectId={selectId}
      label={label}
      isPending={isPending}
      disabled={disabled}
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
    <div className="min-w-0 space-y-1.5">
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
          className="w-full"
          disabled={isPending || disabled}
          aria-describedby={isMissing ? statusId : undefined}
          aria-busy={isPending}
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
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
        <p id={statusId} role="status" className="text-xs text-muted-foreground">
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
  onChange,
}: {
  indexer: IndexerRecord;
  catalog: IndexerDownloadClientMappingCatalog | null;
  options: SeedingProfileOption[];
  isPending: boolean;
  disabled: boolean;
  onChange: (seedingProfileId: string | null) => Promise<void> | void;
}) {
  const t = useTranslate();
  const selectId = selectorId("settings-indexer-seeding-profile", indexer.id);
  if (!catalog) {
    return (
      <span className="text-muted-foreground" data-testid={`${selectId}-loading`}>
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
      onChange={onChange}
    />
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
  indexerProxyConfigs,
  indexerProxyDraft,
  setIndexerProxyDraft,
  editingProxyId,
  isProxyEditorOpen,
  mutatingProxyId,
  testingProxyId,
  submitIndexerProxy,
  resetIndexerProxyDraft,
  startCreateIndexerProxy,
  editIndexerProxy,
  testIndexerProxy,
  deleteIndexerProxy,
  editIndexer,
  toggleIndexerEnabled,
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
  const normalizedProviderType = indexerDraft.providerType.trim().toLowerCase();
  const isManagedSyncProvider = normalizedProviderType === "prowlarr";
  const isEditing = editorMode === "edit";
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
  const proxiesById = React.useMemo(() => {
    return new Map(indexerProxyConfigs.map((proxy) => [proxy.id, proxy]));
  }, [indexerProxyConfigs]);
  const selectedIndexerProxyId = indexerDraft.indexerProxyConfigId;
  const selectedIndexerProxy = selectedIndexerProxyId
    ? proxiesById.get(selectedIndexerProxyId) ?? null
    : null;
  const selectedIndexerProxyMissing =
    Boolean(selectedIndexerProxyId) && !selectedIndexerProxy;
  const selectableIndexerProxies = React.useMemo(() => {
    if (!selectedIndexerProxyId) {
      return indexerProxyConfigs.filter((proxy) => proxy.isEnabled);
    }
    return indexerProxyConfigs.filter(
      (proxy) => proxy.isEnabled || proxy.id === selectedIndexerProxyId,
    );
  }, [indexerProxyConfigs, selectedIndexerProxyId]);
  // Protocol families for the provider the editor is currently on: seeding
  // profiles only apply to torrent-capable indexers.
  const draftProtocolFamilies = React.useMemo(
    () =>
      indexerDownloadClientMappingCatalogResource.catalog?.providerCompatibility.find(
        (entry) =>
          entry.providerType.trim().toLowerCase() === normalizedProviderType,
      )?.protocolFamilies ?? [],
    [indexerDownloadClientMappingCatalogResource.catalog, normalizedProviderType],
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
    () =>
      visibleIndexerConfigFields(
        normalizedProviderType,
        (selectedProvider?.configFields ?? []).filter(
          (field) => field.valueSource !== "HOST_BINDING",
        ),
      ),
    [normalizedProviderType, selectedProvider],
  );

  const handleConfigValueChange = React.useCallback(
    (key: string, value: string) => {
      setIndexerDraft((prev) => ({
        ...prev,
        configValues: applyIndexerConfigOption(
          selectedProviderFields,
          prev.configValues,
          key,
          value,
        ),
      }));
    },
    [selectedProviderFields, setIndexerDraft],
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
        const previousProvider = providerTypes.find(
          (providerType) => providerType.providerType === prev.providerType,
        );
        const shouldAutofillName =
          prev.name.trim().length === 0 ||
          prev.name === (previousProvider?.name ?? prev.providerType);
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
          name: shouldAutofillName ? (nextProvider?.name ?? prev.name) : prev.name,
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

  const showProxies = indexerSettingsTab === "proxies";
  const showIndexers = indexerSettingsTab === "indexers";

  return (
    <div id="settings-indexers-section" className="flex flex-col gap-4 text-sm">
      {showProxies ? (
      <div id="settings-indexer-proxies-panel" className="space-y-4">
      <div id="settings-indexer-proxies-card" className="rounded border border-border">
        <div className="flex items-center justify-between border-b border-border px-3 py-2">
          <CardTitle className="flex items-center gap-2 text-base">
            {t("settings.indexerProxies")}
          </CardTitle>
        </div>
        <div className="overflow-x-auto">
          <Table id="settings-indexer-proxies-table">
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>Provider</TableHead>
                <TableHead>{t("settings.baseUrl")}</TableHead>
                <TableHead className="text-center">{t("label.enabled")}</TableHead>
                <TableHead>Health</TableHead>
                <TableHead>Last error</TableHead>
                <TableHead className="text-right">{t("label.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {indexerProxyConfigs.map((proxy) => (
                <TableRow key={proxy.id} id={selectorId("settings-indexer-proxy-row", proxy.name)} data-ui="settings-table-row">
                  <TableCell className="font-medium">{proxy.name}</TableCell>
                  <TableCell>
                    {proxy.providerType === "trawl" ? "Trawl" : "Byparr"}
                  </TableCell>
                  <TableCell className="max-w-[280px] truncate">{proxy.baseUrl}</TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={proxy.isEnabled}
                      label={`${t("label.enabled")}: ${proxy.name}`}
                    />
                  </TableCell>
                  <TableCell>{formatIndexerProxyHealth(proxy.lastHealthStatus)}</TableCell>
                  <TableCell>
                    {proxy.lastErrorAt ? (
                      <span title={proxy.lastErrorAt}>
                        {formatRelativeTime(proxy.lastErrorAt)}
                      </span>
                    ) : (
                      <span className="text-muted-foreground">—</span>
                    )}
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-2">
                      <IndexerActionButton
                        id={selectorId("settings-indexer-proxy-test", proxy.name)}
                        tone="search"
                        onClick={() => void testIndexerProxy(proxy)}
                        disabled={testingProxyId === proxy.id || mutatingProxyId === proxy.id}
                        label="Test"
                      >
                        <RefreshCw
                          className={cn(
                            "h-4 w-4",
                            testingProxyId === proxy.id && "animate-spin",
                          )}
                        />
                      </IndexerActionButton>
                      <IndexerActionButton
                        id={selectorId("settings-indexer-proxy-edit", proxy.name)}
                        tone="edit"
                        onClick={() => editIndexerProxy(proxy)}
                        disabled={mutatingProxyId !== null}
                        label={t("label.edit")}
                      >
                        <Edit className="h-4 w-4" />
                      </IndexerActionButton>
                      <IndexerActionButton
                        id={selectorId("settings-indexer-proxy-delete", proxy.name)}
                        tone="delete"
                        onClick={() => void deleteIndexerProxy(proxy)}
                        disabled={mutatingProxyId === proxy.id}
                        label={t("label.delete")}
                      >
                        <Trash2 className="h-4 w-4" />
                      </IndexerActionButton>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
              {indexerProxyConfigs.length === 0 ? (
                <TableRow id="settings-indexer-proxies-empty-row">
                  <TableCell colSpan={7} className="text-muted-foreground">
                    No indexer proxies configured.
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </div>
      {isProxyEditorOpen ? (
        <Card>
          <CardHeader>
            <CardTitle className="text-base">
              {editingProxyId ? "Update indexer proxy" : "Connect indexer proxy"}
            </CardTitle>
          </CardHeader>
          <CardContent>
        <form
          id="settings-indexer-proxy-form"
          className="grid gap-3 md:grid-cols-[10rem_minmax(0,1fr)_minmax(0,1.4fr)_10rem_auto_auto]"
          onSubmit={submitIndexerProxy}
        >
          <label>
            <Label className="mb-2 block" htmlFor="settings-indexer-proxy-provider-type">
              Provider
            </Label>
            <Select
              value={indexerProxyDraft.providerType}
              disabled={editingProxyId !== null}
              onValueChange={(value) => {
                if (value !== "byparr" && value !== "trawl") return;
                setIndexerProxyDraft((prev) => ({
                  ...prev,
                  providerType: value,
                }));
              }}
            >
              <SelectTrigger id="settings-indexer-proxy-provider-type" className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="byparr">Byparr</SelectItem>
                <SelectItem value="trawl">Trawl</SelectItem>
              </SelectContent>
            </Select>
          </label>
          <label>
            <Label className="mb-2 block" htmlFor="settings-indexer-proxy-name">
              {t("label.name")}
            </Label>
            <Input
              id="settings-indexer-proxy-name"
              value={indexerProxyDraft.name}
              onChange={(event) =>
                setIndexerProxyDraft((prev) => ({
                  ...prev,
                  name: event.target.value,
                }))
              }
              required
            />
          </label>
          <label>
            <Label className="mb-2 block" htmlFor="settings-indexer-proxy-base-url">
              {t("settings.baseUrl")}
            </Label>
            <Input
              id="settings-indexer-proxy-base-url"
              value={indexerProxyDraft.baseUrl}
              onChange={(event) =>
                setIndexerProxyDraft((prev) => ({
                  ...prev,
                  baseUrl: event.target.value,
                }))
              }
              required
            />
          </label>
          <label>
            <Label className="mb-2 block" htmlFor="settings-indexer-proxy-timeout">
              Timeout
            </Label>
            <Input
              id="settings-indexer-proxy-timeout"
              min={1}
              max={120}
              {...signedIntegerInputProps}
              value={indexerProxyDraft.requestTimeoutSeconds}
              onChange={(event) =>
                setIndexerProxyDraft((prev) => ({
                  ...prev,
                  requestTimeoutSeconds:
                    Number.parseInt(event.target.value, 10) || 1,
                }))
              }
            />
          </label>
          <label className="flex items-center gap-2 self-end pb-2">
            <Checkbox
              id="settings-indexer-proxy-enabled"
              checked={indexerProxyDraft.isEnabled}
              onCheckedChange={(value) =>
                setIndexerProxyDraft((prev) => ({
                  ...prev,
                  isEnabled: value === true,
                }))
              }
            />
            <span>{t("label.enabled")}</span>
          </label>
          <div className="flex items-end gap-2">
            <Button
              id="settings-indexer-proxy-save"
              type="submit"
              disabled={mutatingProxyId !== null}
            >
              {mutatingProxyId
                ? t("label.saving")
                : editingProxyId
                  ? "Update proxy"
                  : "Create proxy"}
            </Button>
            {editingProxyId ? (
              <Button
                id="settings-indexer-proxy-cancel"
                type="button"
                variant="outline"
                onClick={resetIndexerProxyDraft}
                disabled={mutatingProxyId !== null}
              >
                {t("label.cancel")}
              </Button>
            ) : null}
          </div>
        </form>
          </CardContent>
        </Card>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-indexer-proxy-create"
            icon={Plus}
            label="Connect indexer proxy"
            onClick={startCreateIndexerProxy}
            disabled={mutatingProxyId !== null}
          />
        </div>
      )}
      </div>
      ) : null}

      {showIndexers ? (
      <>
      <div id="settings-indexers-table-card" className="rounded border border-border">
        <div className="flex items-center justify-between border-b border-border px-3 py-2">
          <CardTitle className="text-base">
            {t("settings.existingIndexers")}
          </CardTitle>
          <Input
            id="settings-indexers-filter"
            value={settingsIndexerFilter}
            onChange={(event) => setSettingsIndexerFilter(event.target.value)}
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
            className="[&_td]:px-2 [&_th]:px-2"
          >
            <colgroup>
              <col className="w-[13%]" />
              <col className="w-[10%]" />
              <col className="w-[7%]" />
              <col className="w-[16%]" />
              <col className="w-[16%]" />
              <col className="w-[5%]" />
              <col className="w-[6%]" />
              <col className="w-[4%]" />
              <col className="w-[10%]" />
              <col className="w-[13%]" />
            </colgroup>
            <TableHeader>
              <TableRow>
                <TableHead>{t("label.name")}</TableHead>
                <TableHead>{t("settings.indexerProvider")}</TableHead>
                <TableHead>Proxy</TableHead>
                <TableHead>
                  {t("settings.indexerDownloadClient")}
                </TableHead>
                <TableHead>
                  {t("settings.seedingProfileColumn")}
                </TableHead>
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
                <TableHead className="text-right">
                  {t("label.actions")}
                </TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {settingsIndexers.map((indexer) => {
                const parentName = indexer.managedParentConfigId
                  ? indexersById.get(indexer.managedParentConfigId)?.name
                  : null;
                const managedChildCount = managedChildCounts.get(indexer.id) ?? 0;
                const assignedProxy = indexer.indexerProxyConfigId
                  ? proxiesById.get(indexer.indexerProxyConfigId) ?? null
                  : null;
                return (
                <TableRow
                  data-ui="settings-table-row"
                  key={indexer.id}
                  id={selectorId("settings-indexer-row", indexer.name)}
                  className={indexer.isManaged ? "bg-muted/25" : undefined}
                >
                  <TableCell>
                    <div className="space-y-1">
                      <div className="font-medium">{indexer.name}</div>
                      {indexer.isManaged ? (
                        <div className="flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
                          <span className="inline-flex items-center gap-1 rounded-full border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-2 py-0.5 font-medium text-[var(--scry-warning-text)]">
                            <Lock className="h-3 w-3" />
                            {t("settings.managedIndexerBadge")}
                          </span>
                          <span>
                            {parentName
                              ? t("settings.managedByIndexer", { name: parentName })
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
                  </TableCell>
                  <TableCell>
                    <IndexerProviderTypeCell
                      providerType={indexer.providerType}
                    />
                  </TableCell>
                  <TableCell>
                    {assignedProxy ? (
                      <span
                        className={cn(
                          "font-medium",
                          !assignedProxy.isEnabled &&
                            "text-[var(--scry-warning-text)]",
                        )}
                      >
                        {assignedProxy.name}
                      </span>
                    ) : indexer.indexerProxyConfigId ? (
                      <span className="text-[var(--scry-warning-text)]">
                        Missing proxy
                      </span>
                    ) : (
                      <span className="text-muted-foreground">Direct</span>
                    )}
                  </TableCell>
                  <TableCell>
                    <IndexerDownloadClientCell
                      indexer={indexer}
                      resource={indexerDownloadClientMappingCatalogResource}
                      isPending={mutatingIndexerMappingIds.has(indexer.id)}
                      disabled={editingIndexerId === indexer.id && isEditorOpen}
                      onRetry={refreshIndexerDownloadClientMappingCatalog}
                      onChange={(downloadClientId) =>
                        setIndexerDownloadClientMapping(indexer.id, downloadClientId)
                      }
                    />
                  </TableCell>
                  <TableCell>
                    <IndexerSeedingProfileCell
                      indexer={indexer}
                      catalog={indexerDownloadClientMappingCatalogResource.catalog}
                      options={seedingProfileOptions}
                      isPending={mutatingIndexerSeedingProfileIds.has(indexer.id)}
                      disabled={editingIndexerId === indexer.id && isEditorOpen}
                      onChange={(seedingProfileId) =>
                        setIndexerSeedingProfile(indexer.id, seedingProfileId)
                      }
                    />
                  </TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={indexer.isEnabled}
                      label={`${t("label.enabled")}: ${indexer.name}`}
                    />
                  </TableCell>
                  <TableCell className="text-center">
                    {indexer.supportsManagedChildrenSync ? (
                      <span
                        className="text-muted-foreground"
                        title={t("settings.indexerManagedParentHint")}
                      >
                        —
                      </span>
                    ) : (
                      <RenderBooleanIcon
                        value={indexer.enableInteractiveSearch}
                        label={`${t("settings.indexerInteractiveSearch")}: ${indexer.name}`}
                      />
                    )}
                  </TableCell>
                  <TableCell className="text-center">
                    {indexer.supportsManagedChildrenSync ? (
                      <span
                        className="text-muted-foreground"
                        title={t("settings.indexerManagedParentHint")}
                      >
                        —
                      </span>
                    ) : (
                      <RenderBooleanIcon
                        value={indexer.enableAutoSearch}
                        label={`${t("settings.indexerAutoSearch")}: ${indexer.name}`}
                      />
                    )}
                  </TableCell>
                  <TableCell>
                    <IndexerStatusCell
                      indexer={indexer}
                      onOpenErrorHistory={() => setErrorHistoryIndexer({
                        id: indexer.id,
                        name: indexer.name,
                      })}
                    />
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex flex-wrap justify-end gap-2">
                      <IndexerActionButton
                        id={selectorId("settings-indexer-error-history", indexer.name)}
                        tone="search"
                        onClick={() => setErrorHistoryIndexer({
                          id: indexer.id,
                          name: indexer.name,
                        })}
                        label={t("indexerErrors.history")}
                      >
                        <Logs className="h-4 w-4" />
                      </IndexerActionButton>
                      {!indexer.isManaged && indexer.supportsManagedChildrenSync ? (
                        <IndexerActionButton
                          id={selectorId("settings-indexer-sync", indexer.name)}
                          tone="search"
                          onClick={() => void syncIndexer(indexer)}
                          disabled={mutatingIndexerId === indexer.id}
                          label={t("settings.indexerSyncNow")}
                        >
                          <RefreshCw className={cn(
                            "h-4 w-4",
                            mutatingIndexerId === indexer.id && "animate-spin",
                          )} />
                        </IndexerActionButton>
                      ) : null}
                      <IndexerActionButton
                        id={selectorId(
                          "settings-indexer-toggle",
                          indexer.name,
                        )}
                        tone={indexer.isEnabled ? "disabled" : "enabled"}
                        onClick={() => void toggleIndexerEnabled(indexer)}
                        disabled={mutatingIndexerId === indexer.id}
                        label={
                          indexer.isEnabled
                            ? t("label.disable")
                            : t("label.enable")
                        }
                      >
                        {indexer.isEnabled ? (
                          <PowerOff className="h-4 w-4" />
                        ) : (
                          <Power className="h-4 w-4" />
                        )}
                      </IndexerActionButton>
                      {indexer.isManaged ? (
                        <span className="inline-flex items-center gap-1 rounded-full bg-muted px-2 py-1 text-xs text-muted-foreground">
                          <Lock className="h-3 w-3" />
                          {t("settings.managedIndexerBadge")}
                        </span>
                      ) : (
                        <>
                          <IndexerActionButton
                            id={selectorId("settings-indexer-edit", indexer.name)}
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
                );
              })}
              {settingsIndexers.length === 0 ? (
                <TableRow id="settings-indexers-empty-row">
                  <TableCell colSpan={11} className="text-muted-foreground">
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
          <Card>
            <CardHeader>
              <CardTitle className="text-base">
                {editingIndexerId
                  ? t("settings.indexerUpdate")
                  : t("settings.indexerCreate")}
              </CardTitle>
            </CardHeader>
            <CardContent>
              <form id="settings-indexer-form" className="space-y-3" onSubmit={submitIndexer}>
            <div className="grid gap-3 md:grid-cols-2">
              <label>
                <Label className="mb-2 block" htmlFor="settings-indexer-provider-type">
                  {t("form.providerTypePlaceholder")}
                </Label>
                <Select
                  value={normalizedProviderType || undefined}
                  onValueChange={handleProviderTypeChange}
                >
                  <SelectTrigger id="settings-indexer-provider-type" className="w-full">
                    <SelectValue
                      placeholder={t("form.providerTypePlaceholder")}
                    >
                      {normalizedProviderType ? (
                        <PluginVisualLabel
                          providerType={normalizedProviderType}
                          pluginType="indexer"
                          label={
                            providerTypeOptions.find(
                              (option) => option.value === normalizedProviderType,
                            )?.label ?? formatIndexerProviderTypeLabel(normalizedProviderType, t)
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
                <Label className="mb-2 block" htmlFor="settings-indexer-name">{t("label.name")}</Label>
                <Input
                  id="settings-indexer-name"
                  value={indexerDraft.name}
                  onChange={(event) =>
                    setIndexerDraft((prev: IndexerDraft) => ({
                      ...prev,
                      name: event.target.value,
                    }))
                  }
                  required
                  placeholder={t("form.indexerNamePlaceholder")}
                />
              </label>
            </div>

            <div className="grid gap-3 md:grid-cols-2">
            <div className="space-y-2">
              <Label className="block" htmlFor="settings-indexer-proxy-select">
                Indexer proxy
              </Label>
              <Select
                value={selectedIndexerProxyId ?? "none"}
                onValueChange={(value) =>
                  setIndexerDraft((prev: IndexerDraft) => ({
                    ...prev,
                    indexerProxyConfigId: value === "none" ? null : value,
                  }))
                }
              >
                <SelectTrigger id="settings-indexer-proxy-select" className="w-full">
                  <SelectValue placeholder="Direct (no proxy)" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">Direct (no proxy)</SelectItem>
                  {selectedIndexerProxyMissing ? (
                    <SelectItem value={selectedIndexerProxyId ?? "missing"} disabled>
                      Missing proxy
                    </SelectItem>
                  ) : null}
                  {selectableIndexerProxies.map((proxy) => (
                    <SelectItem key={proxy.id} value={proxy.id}>
                      {proxy.name}
                      {proxy.isEnabled ? "" : " (disabled)"}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              {selectedIndexerProxyMissing ? (
                <p className="text-xs text-[var(--scry-warning-text)]">
                  Assigned proxy was not found.
                </p>
              ) : selectedIndexerProxy && !selectedIndexerProxy.isEnabled ? (
                <p className="text-xs text-[var(--scry-warning-text)]">
                  Assigned proxy is disabled.
                </p>
              ) : null}
            </div>
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
                  indexerDownloadClientMappingCatalogResource.status === "error"
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
                resource={indexerDownloadClientMappingCatalogResource}
                selectId="settings-indexer-download-client-form"
                label={t("settings.indexerDownloadClient")}
                showLabel
                onRetry={refreshIndexerDownloadClientMappingCatalog}
              />
            )}
            {indexerDownloadClientMappingCatalogResource.catalog ? (
              <IndexerSeedingProfileSelect
                selectId="settings-indexer-seeding-profile-form"
                label={t("settings.seedingProfileColumn")}
                value={indexerDraft.seedingProfileId}
                options={seedingProfileOptions}
                supported={
                  !isManagedSyncProvider &&
                  supportsSeedingProfileAssignment(draftProtocolFamilies)
                }
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
                <div className="grid gap-3 md:grid-cols-3">
                  {selectedProviderFields
                    .filter((f) => f.fieldType !== "BOOL")
                    .map((field) => (
                      <DynamicConfigField
                        key={field.key}
                        field={field}
                        value={
                          indexerDraft.configValues[field.key] ??
                          field.defaultValue ??
                          ""
                        }
                        hasStoredSecretValue={indexerDraft.storedSecretKeys.includes(
                          field.key,
                        )}
                        onChange={handleConfigValueChange}
                      />
                    ))}
                </div>
                {selectedProviderFields.some((f) => f.fieldType === "BOOL") ? (
                  <div className="flex items-center gap-6">
                    {selectedProviderFields
                      .filter((f) => f.fieldType === "BOOL")
                      .map((field) => (
                        <DynamicConfigField
                          key={field.key}
                          field={field}
                          value={
                            indexerDraft.configValues[field.key] ??
                            field.defaultValue ??
                            "false"
                          }
                          hasStoredSecretValue={indexerDraft.storedSecretKeys.includes(
                            field.key,
                          )}
                          onChange={handleConfigValueChange}
                        />
                      ))}
                  </div>
                ) : null}
              </div>
            ) : null}

            {isManagedSyncProvider ? (
              <p className="text-sm text-muted-foreground">
                {t("settings.indexerManagedParentHint")}
              </p>
            ) : (
              <div className="flex items-center gap-6">
                <label className="flex items-center gap-2">
                  <Checkbox
                    id="settings-indexer-enable-interactive-search"
                    checked={indexerDraft.enableInteractiveSearch}
                    onCheckedChange={(value) =>
                      setIndexerDraft((prev: IndexerDraft) => ({
                        ...prev,
                        enableInteractiveSearch: value === true,
                      }))
                    }
                  />
                  <span className="text-sm">
                    {t("settings.indexerInteractiveSearch")}
                  </span>
                </label>
                <label className="flex items-center gap-2">
                  <Checkbox
                    id="settings-indexer-enable-auto-search"
                    checked={indexerDraft.enableAutoSearch}
                    onCheckedChange={(value) =>
                      setIndexerDraft((prev: IndexerDraft) => ({
                        ...prev,
                        enableAutoSearch: value === true,
                      }))
                    }
                  />
                  <span className="text-sm">
                    {t("settings.indexerAutoSearch")}
                  </span>
                </label>
              </div>
            )}
            <div className="flex gap-2">
              <Button id="settings-indexer-save" type="submit" disabled={mutatingIndexerId === "new"}>
                {mutatingIndexerId === "new"
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
