import { useState } from "react";
import { Check, ChevronRight, Loader2, Search, X } from "lucide-react";
import { DownloadClientConfigField } from "@/components/common/download-client-config-field";
import { PluginVisualLabel } from "@/components/common/plugin-visual";
import { Button } from "@/components/ui/button";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import {
  SetupBackButton,
  SetupPanel,
  SetupPrimaryButton,
  SetupStepHeader,
} from "./setup-chrome";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { type ConfigFieldDef, visibleIndexerConfigFields } from "@/lib/types";
import {
  resolveConfigFieldsForValues,
  splitAdvancedConfigFields,
} from "@/lib/utils/provider-config-fields";
import { cn } from "@/lib/utils";

interface ProviderOption {
  value: string;
  label: string;
  defaultBaseUrl?: string;
  configFields: ConfigFieldDef[];
}

interface SetupIndexerViewProps {
  t: (key: string) => string;
  name: string;
  providerType: string;
  configValues: Record<string, string>;
  providerOptions: ProviderOption[];
  onNameChange: (value: string) => void;
  onProviderTypeChange: (value: string) => void;
  onConfigValueChange: (key: string, value: string) => void;
  onTestConnection: () => void;
  onNext: () => void;
  onBack: () => void;
  onSkip?: () => void;
  testing: boolean;
  testResult: "success" | "failed" | null;
  saving: boolean;
  saved: boolean;
  error: string | null;
}

function isMissingRequiredField(
  field: ConfigFieldDef,
  configValues: Record<string, string>,
) {
  if (!field.required || field.valueSource === "HOST_BINDING") {
    return false;
  }

  const value =
    configValues[field.key] ??
    field.defaultValue ??
    (field.fieldType === "BOOL" ? "false" : "");
  return field.fieldType !== "BOOL" && value.trim() === "";
}

/// The wizard renders the same declarations the settings form does, so it
/// shares that renderer rather than keeping a third copy — this one had no TAG,
/// PATH or FILTERED_SELECT branch and quietly fell through to a text input for
/// each. Only the id prefix is pinned, because selectors depend on it.
function DynamicConfigField({
  field,
  value,
  onChange,
}: {
  field: ConfigFieldDef;
  value: string;
  onChange: (key: string, value: string) => void;
}) {
  return (
    <DownloadClientConfigField
      field={field}
      value={value}
      onChange={onChange}
      idPrefix="setup-indexer-field"
    />
  );
}

/// One group of a provider's fields: values first, then the checkboxes. Used
/// for the standard fields and again inside the advanced disclosure.
function SetupConfigFieldGroup({
  fields,
  configValues,
  onConfigValueChange,
}: {
  fields: ConfigFieldDef[];
  configValues: Record<string, string>;
  onConfigValueChange: (key: string, value: string) => void;
}) {
  if (fields.length === 0) {
    return null;
  }
  const boolFields = fields.filter((field) => field.fieldType === "BOOL");
  const valueFor = (field: ConfigFieldDef, fallback: string) =>
    configValues[field.key] ?? field.defaultValue ?? fallback;

  return (
    <div className="space-y-4">
      {fields
        .filter((field) => field.fieldType !== "BOOL")
        .map((field) => (
          <DynamicConfigField
            key={field.key}
            field={field}
            value={valueFor(field, "")}
            onChange={onConfigValueChange}
          />
        ))}
      {boolFields.length > 0 ? (
        <div className="flex flex-wrap items-center gap-4">
          {boolFields.map((field) => (
            <DynamicConfigField
              key={field.key}
              field={field}
              value={valueFor(field, "false")}
              onChange={onConfigValueChange}
            />
          ))}
        </div>
      ) : null}
    </div>
  );
}

export function SetupIndexerView({
  t,
  name,
  providerType,
  configValues,
  providerOptions,
  onNameChange,
  onProviderTypeChange,
  onConfigValueChange,
  onTestConnection,
  onNext,
  onBack,
  onSkip,
  testing,
  testResult,
  saving,
  saved,
  error,
}: SetupIndexerViewProps) {
  const selectedProvider = providerOptions.find((p) => p.value === providerType);
  const selectedProviderFields = visibleIndexerConfigFields(
    selectedProvider?.configFields ?? [],
  );
  const { standard: standardProviderFields, advanced: advancedProviderFields } =
    splitAdvancedConfigFields(
      resolveConfigFieldsForValues(selectedProviderFields, configValues),
    );
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const hasMissingRequiredField = selectedProviderFields.some((field) =>
    isMissingRequiredField(field, configValues),
  );
  const canTest =
    name.trim().length > 0 && providerType.length > 0 && !hasMissingRequiredField;
  const canProceed = saved;

  return (
    <SetupPanel id="setup-indexer-view" className="flex flex-col gap-6">
      <SetupStepHeader
        icon={Search}
        title={t("setup.indexerTitle")}
        subtitle={t("setup.indexerDescription")}
      />
      <div className="mx-auto flex w-full max-w-md flex-col gap-4">
        <div className="space-y-2">
          <Label htmlFor="setup-indexer-name">{t("label.name")}</Label>
          <Input
            id="setup-indexer-name"
            value={name}
            onChange={(e) => onNameChange(e.target.value)}
            placeholder="My Indexer"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="setup-indexer-provider">{t("settings.indexerProvider")}</Label>
          <Select value={providerType} onValueChange={onProviderTypeChange}>
            <SelectTrigger id="setup-indexer-provider">
              <SelectValue placeholder="Select provider">
                {selectedProvider ? (
                  <PluginVisualLabel
                    providerType={selectedProvider.value}
                    pluginType="indexer"
                    label={selectedProvider.label}
                  />
                ) : null}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              {providerOptions.map((opt) => (
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
        </div>
        <SetupConfigFieldGroup
          fields={standardProviderFields}
          configValues={configValues}
          onConfigValueChange={onConfigValueChange}
        />
        {advancedProviderFields.length > 0 ? (
          <Collapsible open={advancedOpen} onOpenChange={setAdvancedOpen}>
            <CollapsibleTrigger asChild>
              <button
                id="setup-indexer-advanced-toggle"
                type="button"
                className="flex items-center gap-1.5 rounded-[8px] py-1 text-sm font-medium text-[var(--scry-muted)] transition-colors hover:text-[var(--scry-ink2)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <ChevronRight
                  className={cn(
                    "h-4 w-4 transition-transform",
                    advancedOpen && "rotate-90",
                  )}
                />
                {t("settings.advancedConfig")}
                <span className="text-[var(--scry-faint)]">
                  ({advancedProviderFields.length})
                </span>
              </button>
            </CollapsibleTrigger>
            <CollapsibleContent className="space-y-4 pt-3">
              <SetupConfigFieldGroup
                fields={advancedProviderFields}
                configValues={configValues}
                onConfigValueChange={onConfigValueChange}
              />
            </CollapsibleContent>
          </Collapsible>
        ) : null}
        <div className="flex items-center gap-3">
          <Button
            id="setup-indexer-test-connection"
            variant="outline"
            onClick={onTestConnection}
            disabled={!canTest || testing || saving}
          >
            {testing ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : null}
            {t("label.testConnection")}
          </Button>
          {testResult === "success" && (
            <span
              id="setup-indexer-test-result-success"
              className="flex items-center gap-1 text-sm text-[var(--scry-success-text-soft)]"
            >
              <Check className="h-4 w-4" /> {t("setup.connectionSuccess")}
            </span>
          )}
          {testResult === "failed" && (
            <span
              id="setup-indexer-test-result-failed"
              className="flex items-center gap-1 text-sm text-destructive"
            >
              <X className="h-4 w-4" /> {t("setup.connectionFailed")}
            </span>
          )}
        </div>
        {error && <p id="setup-indexer-error" className="text-sm text-destructive">{error}</p>}
        {saved && (
          <p id="setup-indexer-saved" className="text-sm text-[var(--scry-success-text-soft)]">{t("setup.saved")}</p>
        )}
      </div>
      <div className="flex items-center justify-between pt-2">
        <SetupBackButton id="setup-indexer-back" onClick={onBack}>
          {t("setup.back")}
        </SetupBackButton>
        <div className="flex items-center gap-3">
          {onSkip && (
            <Button id="setup-indexer-skip" type="button" variant="link" onClick={onSkip}>
              {t("setup.skip")}
            </Button>
          )}
          <SetupPrimaryButton id="setup-indexer-next" onClick={onNext} disabled={!canProceed || saving}>
            {saving ? t("label.saving") : t("setup.next")}
          </SetupPrimaryButton>
        </div>
      </div>
    </SetupPanel>
  );
}
