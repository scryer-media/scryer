import * as React from "react";
import {
  CircleAlert,
  Edit,
  Loader2,
  Plus,
  PlugZap,
  Power,
  PowerOff,
  Trash2,
} from "lucide-react";
import { AddNewButton } from "@/components/common/add-new-button";
import { PluginVisualLabel } from "@/components/common/plugin-visual";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
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
  TableActionsCell,
  TableActionsHead,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  ConfigFieldDef,
  SubtitleProviderConfigRecord,
  SubtitleProviderDraft,
  SubtitleProviderTypeInfo,
} from "@/lib/types";
import type { BoxedActionButtonTone } from "@/lib/utils/action-button-styles";
import { selectorId } from "@/lib/utils/dom-ids";

type Props = {
  editingProviderId: string | null;
  providerDraft: SubtitleProviderDraft;
  setProviderDraft: React.Dispatch<React.SetStateAction<SubtitleProviderDraft>>;
  submitProvider: (
    event: React.FormEvent<HTMLFormElement>,
  ) => Promise<void> | void;
  mutatingProviderId: string | null;
  resetProviderDraft: () => void;
  providerConfigs: SubtitleProviderConfigRecord[];
  editProvider: (provider: SubtitleProviderConfigRecord) => void;
  toggleProviderEnabled: (
    provider: SubtitleProviderConfigRecord,
  ) => Promise<void> | void;
  deleteProvider: (provider: SubtitleProviderConfigRecord) => Promise<void> | void;
  providerTypes: SubtitleProviderTypeInfo[];
  testProviderConnection: () => Promise<void> | void;
  isTestingConnection: boolean;
  isEditorOpen: boolean;
  editorMode: "create" | "edit";
  startCreateProvider: () => void;
};

const SUBTITLE_FACETS = [
  { value: "MOVIE", labelKey: "label.movies" },
  { value: "SERIES", labelKey: "label.series" },
  { value: "ANIME", labelKey: "label.anime" },
] as const;

const PROVIDER_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const PROVIDER_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const PROVIDER_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const PROVIDER_PANEL_BODY_CLASS = "p-4 sm:p-5";
const PROVIDER_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";
const PROVIDER_TABLE_HEADER_CELL_CLASS =
  "text-center font-semibold text-[var(--scry-muted3)]";

function looksLikeSecretConfigKey(key: string): boolean {
  const normalized = key.trim().toLowerCase();
  return (
    normalized === "api_key" ||
    normalized === "apikey" ||
    normalized.includes("api_key") ||
    normalized.includes("password") ||
    normalized.includes("secret") ||
    normalized.includes("token")
  );
}

function SubtitleProviderActionButton({
  label,
  tone,
  className,
  children,
  ...props
}: Omit<React.ComponentProps<typeof IconButton>, "tone"> & {
  label: string;
  tone: Extract<BoxedActionButtonTone, "edit" | "enabled" | "disabled" | "delete">;
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

function SubtitleProviderStatusCell({
  provider,
}: {
  provider: SubtitleProviderConfigRecord;
}) {
  const t = useTranslate();
  if (!provider.isEnabled) {
    return <span className={PROVIDER_MUTED_TEXT_CLASS}>{t("label.disabled")}</span>;
  }

  if (provider.disabledUntil) {
    const until = new Date(provider.disabledUntil);
    if (until > new Date()) {
      return (
        <span
          className="text-[var(--scry-warning-text)]"
          title={provider.disabledUntil}
        >
          {t("settings.subtitleProviderDisabledUntil", {
            time: formatRelativeTime(provider.disabledUntil),
          })}
        </span>
      );
    }
  }

  if (provider.lastErrorAt) {
    return (
      <div className="space-y-1">
        <span
          className="text-[var(--scry-danger-text-soft)]"
          title={provider.lastErrorAt}
        >
          {t("settings.subtitleProviderLastError", {
            time: formatRelativeTime(provider.lastErrorAt),
          })}
        </span>
        {provider.lastError ? (
          <p className={`max-w-sm text-xs ${PROVIDER_MUTED_TEXT_CLASS}`}>
            {provider.lastError}
          </p>
        ) : null}
      </div>
    );
  }

  if (provider.lastHealthStatus) {
    return (
      <span className={PROVIDER_MUTED_TEXT_CLASS}>
        {provider.lastHealthStatus}
      </span>
    );
  }

  return (
    <span className={PROVIDER_MUTED_TEXT_CLASS}>
      {t("settings.subtitleProviderNoActivity")}
    </span>
  );
}

function SubtitleProviderFacetChips({
  facets,
}: {
  facets: SubtitleProviderConfigRecord["enabledFacets"];
}) {
  const t = useTranslate();
  if (facets.length === 0) {
    return <span className={PROVIDER_MUTED_TEXT_CLASS}>-</span>;
  }

  return (
    <div className="flex flex-wrap gap-1">
      {facets.map((facet) => {
        const labelKey =
          SUBTITLE_FACETS.find((item) => item.value === facet)?.labelKey ??
          "label.unknown";
        return (
          <span
            key={facet}
            className="rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-0.5 text-[11px] text-[var(--scry-ink2)]"
          >
            {t(labelKey)}
          </span>
        );
      })}
    </div>
  );
}


function DynamicSubtitleConfigField({
  field,
  value,
  onChange,
  hasStoredSecretValue,
}: {
  field: ConfigFieldDef;
  value: string;
  onChange: (key: string, value: string) => void;
  hasStoredSecretValue: boolean;
}) {
  const t = useTranslate();
  const fieldId = selectorId("settings-subtitle-provider-config", field.key);
  const requiredMarker = field.required ? (
    <span aria-hidden="true" className="text-destructive">
      *
    </span>
  ) : null;

  if (field.fieldType === "BOOL") {
    return (
      <label className="flex items-center gap-2 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2">
        <Checkbox
          id={fieldId}
          checked={value === "true"}
          onCheckedChange={(checkedValue) =>
            onChange(field.key, checkedValue === true ? "true" : "false")
          }
        />
        <div className="space-y-1">
          <span className="inline-flex items-center gap-2 text-sm font-medium text-[var(--scry-ink2)]">
            {field.label}
            {requiredMarker}
          </span>
          {field.helpText ? (
            <p className={`text-xs ${PROVIDER_MUTED_TEXT_CLASS}`}>
              {field.helpText}
            </p>
          ) : null}
        </div>
      </label>
    );
  }

  if (field.fieldType === "SELECT" && field.options.length > 0) {
    return (
      <label>
        <Label
          htmlFor={fieldId}
          className="mb-2 inline-flex items-center gap-2 text-[var(--scry-ink2)]"
        >
          {field.label}
          {requiredMarker}
        </Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(nextValue) => onChange(field.key, nextValue)}
        >
          <SelectTrigger id={fieldId} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                {option.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        {field.helpText ? (
          <p className={`mt-1 text-xs ${PROVIDER_MUTED_TEXT_CLASS}`}>
            {field.helpText}
          </p>
        ) : null}
      </label>
    );
  }

  if (field.fieldType === "MULTILINE") {
    return (
      <label>
        <Label
          htmlFor={fieldId}
          className="mb-2 inline-flex items-center gap-2 text-[var(--scry-ink2)]"
        >
          {field.label}
          {requiredMarker}
        </Label>
        <Textarea
          id={fieldId}
          value={value}
          onChange={(event) => onChange(field.key, event.target.value)}
          required={field.required}
          placeholder={field.defaultValue ?? ""}
          rows={6}
        />
        {field.helpText ? (
          <p className={`mt-1 text-xs ${PROVIDER_MUTED_TEXT_CLASS}`}>
            {field.helpText}
          </p>
        ) : null}
      </label>
    );
  }

  const isSecretField =
    field.fieldType === "PASSWORD" || looksLikeSecretConfigKey(field.key);

  return (
    <label>
      <Label
        htmlFor={fieldId}
        className="mb-2 inline-flex items-center gap-2 text-[var(--scry-ink2)]"
      >
        {field.label}
        {requiredMarker}
      </Label>
      <Input
        id={fieldId}
        value={value}
        onChange={(event) => onChange(field.key, event.target.value)}
        type={isSecretField ? "password" : "text"}
        ignorePasswordManagers={isSecretField}
        required={field.required && !hasStoredSecretValue}
        placeholder={
          isSecretField && hasStoredSecretValue
            ? t("settings.subtitleProviderSecretStored")
            : (field.defaultValue ?? "")
        }
      />
      {field.helpText ? (
        <p className={`mt-1 text-xs ${PROVIDER_MUTED_TEXT_CLASS}`}>
          {field.helpText}
        </p>
      ) : null}
    </label>
  );
}

export function SettingsSubtitleProvidersSection({
  editingProviderId,
  providerDraft,
  setProviderDraft,
  submitProvider,
  mutatingProviderId,
  resetProviderDraft,
  providerConfigs,
  editProvider,
  toggleProviderEnabled,
  deleteProvider,
  providerTypes,
  testProviderConnection,
  isTestingConnection,
  isEditorOpen,
  editorMode,
  startCreateProvider,
}: Props) {
  const t = useTranslate();
  const normalizedProviderType = providerDraft.providerType.trim().toLowerCase();
  const isEditing = editorMode === "edit";

  const providerTypeOptions = React.useMemo(() => {
    const baseOptions = providerTypes.map((providerType) => ({
      value: providerType.providerType,
      label: providerType.name,
    }));

    if (!normalizedProviderType) {
      return baseOptions;
    }

    if (baseOptions.some((option) => option.value === normalizedProviderType)) {
      return baseOptions;
    }

    return [
      { value: normalizedProviderType, label: providerDraft.providerType },
      ...baseOptions,
    ];
  }, [normalizedProviderType, providerDraft.providerType, providerTypes]);

  const selectedProvider = React.useMemo(
    () =>
      providerTypes.find(
        (providerType) => providerType.providerType === normalizedProviderType,
      ) ?? null,
    [normalizedProviderType, providerTypes],
  );

  const selectedProviderFields = React.useMemo(
    () =>
      (selectedProvider?.configFields ?? []).filter(
        (field) => field.valueSource !== "HOST_BINDING",
      ),
    [selectedProvider],
  );

  const handleProviderTypeChange = React.useCallback(
    (nextProviderType: string) => {
      const nextProvider = providerTypes.find(
        (providerType) => providerType.providerType === nextProviderType,
      );
      setProviderDraft((previous) => {
        const previousProvider = providerTypes.find(
          (providerType) => providerType.providerType === previous.providerType,
        );
        const shouldAutofillName =
          previous.name.trim().length === 0 ||
          previous.name === (previousProvider?.name ?? previous.providerType);
        const nextConfigValues: Record<string, string> = {};
        for (const field of nextProvider?.configFields ?? []) {
          if (field.valueSource === "HOST_BINDING") {
            continue;
          }
          nextConfigValues[field.key] =
            previous.persistedConfigValues[field.key] ??
            field.defaultValue ??
            (field.fieldType === "BOOL" ? "false" : "");
        }
        return {
          ...previous,
          providerType: nextProviderType,
          name: shouldAutofillName
            ? (nextProvider?.name ?? previous.name)
            : previous.name,
          configValues: nextConfigValues,
          persistedConfigValues: {},
          storedSecretKeys: [],
          configDirty: true,
          enabledFacets: nextProvider?.recommendedFacets ?? [],
        };
      });
    },
    [providerTypes, setProviderDraft],
  );

  const handleConfigValueChange = React.useCallback(
    (key: string, value: string) => {
      setProviderDraft((previous) => ({
        ...previous,
        configValues: {
          ...previous.configValues,
          [key]: value,
        },
        configDirty: true,
      }));
    },
    [setProviderDraft],
  );

  const handleFacetToggle = React.useCallback(
    (facet: "MOVIE" | "SERIES" | "ANIME", checked: boolean) => {
      setProviderDraft((previous) => {
        const current = new Set(previous.enabledFacets);
        if (checked) {
          current.add(facet);
        } else {
          current.delete(facet);
        }
        return {
          ...previous,
          enabledFacets: SUBTITLE_FACETS.map((item) => item.value).filter((value) =>
            current.has(value),
          ),
        };
      });
    },
    [setProviderDraft],
  );

  return (
    <div id="settings-subtitle-providers-section" className="space-y-4">
      <section className={PROVIDER_PANEL_CLASS}>
        <div className={PROVIDER_PANEL_HEADER_CLASS}>
          <h2 className={`flex items-center gap-2 ${PROVIDER_PANEL_TITLE_CLASS}`}>
            <PlugZap className="h-4 w-4" />
            {t("settings.existingSubtitleProviders")}
          </h2>
        </div>
        <div className="overflow-hidden">
          <Table overflow="clip" layout="fixed" density="dense">
            <TableHeader>
              <TableRow className="border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]">
                <TableHead className={`w-[24%] font-semibold ${PROVIDER_MUTED_TEXT_CLASS}`}>{t("label.name")}</TableHead>
                <TableHead className={PROVIDER_TABLE_HEADER_CELL_CLASS}>{t("settings.subtitleProviderType")}</TableHead>
                <TableHead className={`w-24 ${PROVIDER_TABLE_HEADER_CELL_CLASS}`}>{t("label.enabled")}</TableHead>
                <TableHead className={PROVIDER_TABLE_HEADER_CELL_CLASS}>{t("settings.subtitleProviderFacets")}</TableHead>
                <TableHead className={PROVIDER_TABLE_HEADER_CELL_CLASS}>{t("settings.subtitleProviderStatus")}</TableHead>
                <TableActionsHead className="w-36">{t("label.actions")}</TableActionsHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {providerConfigs.length === 0 ? (
                <TableRow>
                  <TableCell
                    colSpan={6}
                    className={`text-center text-sm ${PROVIDER_MUTED_TEXT_CLASS}`}
                  >
                    {t("settings.subtitleProviderEmpty")}
                  </TableCell>
                </TableRow>
              ) : (
                providerConfigs.map((provider) => (
                  <TableRow
                    data-ui="settings-table-row"
                    key={provider.id}
                    id={selectorId("settings-subtitle-provider-row", provider.name)}
                    className="border-[var(--scry-border3)] hover:bg-[var(--scry-rowHover)]"
                  >
                    <TableCell className="truncate font-medium text-[var(--scry-ink2)]">{provider.name}</TableCell>
                    <TableCell className="text-center text-[var(--scry-ink2)]">
                      <span className="inline-flex justify-center">
                        <PluginVisualLabel
                          providerType={provider.providerType}
                          pluginType="subtitle_provider"
                          label={provider.providerType}
                        />
                      </span>
                    </TableCell>
                    <TableCell className="text-center">
                      <span className={provider.isEnabled ? "text-[var(--scry-success-text-soft)]" : PROVIDER_MUTED_TEXT_CLASS}>
                        {provider.isEnabled ? t("label.enabled") : t("label.disabled")}
                      </span>
                    </TableCell>
                    <TableCell className="text-center">
                      <SubtitleProviderFacetChips facets={provider.enabledFacets ?? []} />
                    </TableCell>
                    <TableCell className="text-center">
                      <SubtitleProviderStatusCell provider={provider} />
                    </TableCell>
                    <TableActionsCell className="w-36">
                      <div className="inline-flex items-center justify-center gap-2">
                        <SubtitleProviderActionButton
                          id={selectorId("settings-subtitle-provider-edit", provider.id)}
                          label={t("label.edit")}
                          tone="edit"
                          onClick={() => editProvider(provider)}
                          disabled={mutatingProviderId !== null}
                        >
                          <Edit className="h-4 w-4" />
                        </SubtitleProviderActionButton>
                        <SubtitleProviderActionButton
                          id={selectorId("settings-subtitle-provider-toggle", provider.id)}
                          label={
                            provider.isEnabled ? t("label.disable") : t("label.enable")
                          }
                          tone={provider.isEnabled ? "enabled" : "disabled"}
                          onClick={() => void toggleProviderEnabled(provider)}
                          disabled={mutatingProviderId !== null}
                        >
                          {provider.isEnabled ? (
                            <Power className="h-4 w-4" />
                          ) : (
                            <PowerOff className="h-4 w-4" />
                          )}
                        </SubtitleProviderActionButton>
                        <SubtitleProviderActionButton
                          id={selectorId("settings-subtitle-provider-delete", provider.id)}
                          label={t("label.delete")}
                          tone="delete"
                          onClick={() => void deleteProvider(provider)}
                          disabled={mutatingProviderId !== null}
                        >
                          <Trash2 className="h-4 w-4" />
                        </SubtitleProviderActionButton>
                      </div>
                    </TableActionsCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </section>

      {isEditorOpen ? (
        <>
          <form
            id="settings-subtitle-provider-form"
            className={PROVIDER_PANEL_CLASS}
            onSubmit={submitProvider}
          >
            <div className={`${PROVIDER_PANEL_HEADER_CLASS} flex items-center justify-between gap-3`}>
              <h2 className={PROVIDER_PANEL_TITLE_CLASS}>
                {isEditing
                  ? t("settings.subtitleProviderEdit")
                  : t("settings.subtitleProviderCreate")}
              </h2>
              {mutatingProviderId ? (
                <Loader2 className={`h-4 w-4 animate-spin ${PROVIDER_MUTED_TEXT_CLASS}`} />
              ) : null}
            </div>

            <div className={`${PROVIDER_PANEL_BODY_CLASS} space-y-4`}>
            <div className="grid gap-4 md:grid-cols-2">
              <label>
                <Label className="mb-2 block text-[var(--scry-ink2)]">{t("settings.subtitleProviderType")}</Label>
                <Select
                  value={normalizedProviderType}
                  onValueChange={handleProviderTypeChange}
                >
                  <SelectTrigger id="settings-subtitle-provider-type" className="w-full">
                    <SelectValue placeholder={t("form.providerTypePlaceholder")}>
                      {normalizedProviderType ? (
                        <PluginVisualLabel
                          providerType={normalizedProviderType}
                          pluginType="subtitle_provider"
                          label={
                            selectedProvider?.name ??
                            providerTypeOptions.find(
                              (option) => option.value === normalizedProviderType,
                            )?.label ??
                            normalizedProviderType
                          }
                        />
                      ) : null}
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    {providerTypeOptions.map((option) => (
                      <SelectItem
                        id={selectorId(
                          "settings-subtitle-provider-type-option",
                          option.value,
                        )}
                        key={option.value}
                        value={option.value}
                      >
                        <PluginVisualLabel
                          providerType={option.value}
                          pluginType="subtitle_provider"
                          label={option.label}
                        />
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>

              <label>
                <Label className="mb-2 block text-[var(--scry-ink2)]">{t("label.name")}</Label>
                <Input
                  id="settings-subtitle-provider-name"
                  value={providerDraft.name}
                  onChange={(event) =>
                    setProviderDraft((previous) => ({
                      ...previous,
                      name: event.target.value,
                    }))
                  }
                  placeholder={t("settings.subtitleProviderNamePlaceholder")}
                  required
                />
              </label>
            </div>

            <label className="flex items-center gap-2 text-[var(--scry-ink2)]">
              <Checkbox
                id="settings-subtitle-provider-enabled"
                checked={providerDraft.isEnabled}
                onCheckedChange={(value) =>
                  setProviderDraft((previous) => ({
                    ...previous,
                    isEnabled: value === true,
                  }))
                }
              />
              <span className="text-sm">{t("label.enabled")}</span>
            </label>

            <div className="space-y-2">
              <Label className="block text-[var(--scry-ink2)]">{t("settings.subtitleProviderFacets")}</Label>
              <div className="flex flex-wrap gap-3">
                {SUBTITLE_FACETS.map((facet) => (
                  <label
                    key={facet.value}
                    className="flex items-center gap-2 rounded-[10px] border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-3 py-2 text-sm text-[var(--scry-ink2)]"
                  >
                    <Checkbox
                      id={selectorId("settings-subtitle-provider-facet", facet.value)}
                      checked={providerDraft.enabledFacets.includes(facet.value)}
                      onCheckedChange={(value) =>
                        handleFacetToggle(facet.value, value === true)
                      }
                    />
                    <span>{t(facet.labelKey)}</span>
                  </label>
                ))}
              </div>
              <p className={`text-xs ${PROVIDER_MUTED_TEXT_CLASS}`}>
                {t("settings.subtitleProviderFacetsHelp")}
              </p>
            </div>

            {selectedProviderFields.length > 0 ? (
              <div className="grid gap-4 md:grid-cols-2">
                {selectedProviderFields.map((field) => (
                  <DynamicSubtitleConfigField
                    key={`${normalizedProviderType}:${field.key}`}
                    field={field}
                    value={providerDraft.configValues[field.key] ?? ""}
                    onChange={handleConfigValueChange}
                    hasStoredSecretValue={providerDraft.storedSecretKeys.includes(
                      field.key,
                    )}
                  />
                ))}
              </div>
            ) : normalizedProviderType ? (
              <div className="rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2 text-sm text-[var(--scry-warning-text)]">
                <div className="flex items-start gap-2">
                  <CircleAlert className="mt-0.5 h-4 w-4 shrink-0 text-[var(--scry-warning-text)]" />
                  <p>{t("settings.subtitleProviderUnknownType")}</p>
                </div>
              </div>
            ) : null}

            <div className="flex flex-wrap gap-2">
              <Button
                id="settings-subtitle-provider-save"
                type="submit"
                disabled={
                  mutatingProviderId !== null ||
                  !providerDraft.name.trim() ||
                  !normalizedProviderType
                }
              >
                {editingProviderId ? t("label.save") : t("label.create")}
              </Button>
              <Button
                id="settings-subtitle-provider-test-connection"
                type="button"
                variant="secondary"
                onClick={() => void testProviderConnection()}
                disabled={isTestingConnection || !normalizedProviderType}
              >
                {isTestingConnection ? (
                  <Loader2 className="mr-1 h-4 w-4 animate-spin" />
                ) : null}
                {t("label.testConnection")}
              </Button>
              <Button
                id="settings-subtitle-provider-cancel"
                type="button"
                variant="outline"
                onClick={resetProviderDraft}
                disabled={mutatingProviderId !== null}
              >
                {t("label.cancel")}
              </Button>
            </div>
            </div>
          </form>
          {isEditing ? (
            <div className="flex justify-center">
              <AddNewButton
                id="settings-subtitle-provider-create-new"
                icon={Plus}
                label={t("settings.subtitleProviderCreateNew")}
                onClick={startCreateProvider}
                disabled={mutatingProviderId !== null}
              />
            </div>
          ) : null}
        </>
      ) : (
        <div className="flex justify-center">
          <AddNewButton
            id="settings-subtitle-provider-create"
            icon={Plus}
            label={t("settings.subtitleProviderCreateNew")}
            onClick={startCreateProvider}
          />
        </div>
      )}
    </div>
  );
}
