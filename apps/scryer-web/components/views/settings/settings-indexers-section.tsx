
import * as React from "react";
import { Edit, MonitorCog, Power, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { RenderBooleanIcon } from "@/components/common/boolean-icon";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { Translate } from "@/components/root/types";
import type { IndexerRecord, IndexerDraft, ProviderTypeInfo, ConfigFieldDef } from "@/lib/types";

type SettingsIndexersSectionProps = {
  t: Translate;
  editingIndexerId: string | null;
  indexerDraft: IndexerDraft;
  setIndexerDraft: React.Dispatch<React.SetStateAction<IndexerDraft>>;
  submitIndexer: (event: React.FormEvent<HTMLFormElement>) => Promise<void> | void;
  mutatingIndexerId: string | null;
  resetIndexerDraft: () => void;
  settingsIndexerFilter: string;
  setSettingsIndexerFilter: (value: string) => void;
  settingsIndexers: IndexerRecord[];
  editIndexer: (indexer: IndexerRecord) => void;
  toggleIndexerEnabled: (indexer: IndexerRecord) => Promise<void> | void;
  deleteIndexer: (indexer: IndexerRecord) => Promise<void> | void;
  providerTypes: ProviderTypeInfo[];
};

const FALLBACK_PROVIDER_OPTIONS = [
  { value: "nzbgeek", label: "Nzbgeek" },
  { value: "dognzb", label: "DogNZB" },
];

const INDEXER_PROVIDER_LOGOS: Record<string, string> = {
  nzbgeek: "/media-sites/nzbgeek.svg",
};

function getProviderLogoSrc(value: string) {
  return INDEXER_PROVIDER_LOGOS[value.trim().toLowerCase()];
}

function IndexerProviderTypeCell({ providerType }: { providerType: string }) {
  const logoSrc = getProviderLogoSrc(providerType);
  return (
    <div className="inline-flex items-center gap-2">
      {logoSrc ? (
        <img
          src={logoSrc}
          alt=""
          aria-hidden="true"
          className="h-4 w-4 object-contain"
        />
      ) : null}
      <span>{providerType}</span>
    </div>
  );
}

function DynamicConfigField({
  field,
  value,
  onChange,
}: {
  field: ConfigFieldDef;
  value: string;
  onChange: (key: string, value: string) => void;
}) {
  if (field.fieldType === "bool") {
    return (
      <label className="flex items-center gap-2">
        <input
          type="checkbox"
          checked={value === "true"}
          onChange={(e) => onChange(field.key, e.target.checked ? "true" : "false")}
          className="accent-primary"
        />
        <span className="text-sm">{field.label}</span>
        {field.helpText ? (
          <span className="text-xs text-muted-foreground">{field.helpText}</span>
        ) : null}
      </label>
    );
  }

  if (field.fieldType === "select" && field.options.length > 0) {
    return (
      <label>
        <Label className="mb-2 block">{field.label}</Label>
        <Select
          value={value || field.defaultValue || ""}
          onValueChange={(v) => onChange(field.key, v)}
        >
          <SelectTrigger className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {field.options.map((opt) => (
              <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
            ))}
          </SelectContent>
        </Select>
        {field.helpText ? (
          <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
        ) : null}
      </label>
    );
  }

  return (
    <label>
      <Label className="mb-2 block">{field.label}</Label>
      <Input
        value={value}
        onChange={(e) => onChange(field.key, e.target.value)}
        type={field.fieldType === "password" ? "password" : field.fieldType === "number" ? "number" : "text"}
        required={field.required}
        placeholder={field.defaultValue ?? ""}
      />
      {field.helpText ? (
        <p className="mt-1 text-xs text-muted-foreground">{field.helpText}</p>
      ) : null}
    </label>
  );
}

export function SettingsIndexersSection({
  t,
  editingIndexerId,
  indexerDraft,
  setIndexerDraft,
  submitIndexer,
  mutatingIndexerId,
  resetIndexerDraft,
  settingsIndexerFilter,
  setSettingsIndexerFilter,
  settingsIndexers,
  editIndexer,
  toggleIndexerEnabled,
  deleteIndexer,
  providerTypes,
}: SettingsIndexersSectionProps) {
  const normalizedProviderType = indexerDraft.providerType.trim().toLowerCase();

  // Build provider type options from loaded plugins, falling back to hardcoded list
  const providerTypeOptions = React.useMemo(() => {
    const baseOptions = providerTypes.length > 0
      ? providerTypes.map((pt) => ({ value: pt.providerType, label: pt.name }))
      : FALLBACK_PROVIDER_OPTIONS;

    if (!normalizedProviderType) {
      return baseOptions;
    }
    if (baseOptions.some((option) => option.value === normalizedProviderType)) {
      return baseOptions;
    }
    return [{ value: normalizedProviderType, label: indexerDraft.providerType }, ...baseOptions];
  }, [indexerDraft.providerType, normalizedProviderType, providerTypes]);

  // Get config fields for the selected provider type
  const selectedProviderFields = React.useMemo(() => {
    const match = providerTypes.find(
      (pt) => pt.providerType === normalizedProviderType,
    );
    return match?.configFields ?? [];
  }, [normalizedProviderType, providerTypes]);

  const handleConfigValueChange = React.useCallback(
    (key: string, value: string) => {
      setIndexerDraft((prev) => ({
        ...prev,
        configValues: { ...prev.configValues, [key]: value },
      }));
    },
    [setIndexerDraft],
  );

  return (
    <div className="space-y-4 text-sm">
      <CardTitle className="flex items-center gap-2 text-base">
        <MonitorCog className="h-4 w-4" />
        {t("settings.indexerProviderSection")}
      </CardTitle>

      <div className="rounded border border-border">
        <div className="flex items-center justify-between border-b border-border px-3 py-2">
          <CardTitle className="text-base">{t("settings.existingIndexers")}</CardTitle>
          <Input
            value={settingsIndexerFilter}
            onChange={(event) => setSettingsIndexerFilter(event.target.value)}
            placeholder={t("settings.indexerFilterPlaceholder")}
            className="max-w-64"
          />
        </div>
        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("settings.indexerName")}</TableHead>
                <TableHead>{t("settings.indexerProvider")}</TableHead>
                <TableHead>{t("settings.indexerBaseUrl")}</TableHead>
                <TableHead className="text-center">{t("label.enabled")}</TableHead>
                <TableHead className="text-center">{t("settings.indexerInteractiveSearch")}</TableHead>
                <TableHead className="text-center">{t("settings.indexerAutoSearch")}</TableHead>
                <TableHead>{t("settings.indexerStatus")}</TableHead>
                <TableHead className="text-right">{t("settings.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {settingsIndexers.map((indexer) => (
                <TableRow key={indexer.id}>
                  <TableCell>{indexer.name}</TableCell>
                  <TableCell>
                    <IndexerProviderTypeCell providerType={indexer.providerType} />
                  </TableCell>
                  <TableCell className="max-w-[260px] truncate">{indexer.baseUrl}</TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={indexer.isEnabled}
                      label={`${t("label.enabled")}: ${indexer.name}`}
                    />
                  </TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={indexer.enableInteractiveSearch}
                      label={`${t("settings.indexerInteractiveSearch")}: ${indexer.name}`}
                    />
                  </TableCell>
                  <TableCell className="text-center">
                    <RenderBooleanIcon
                      value={indexer.enableAutoSearch}
                      label={`${t("settings.indexerAutoSearch")}: ${indexer.name}`}
                    />
                  </TableCell>
                  <TableCell>
                    <span className="italic text-muted-foreground">not implemented yet</span>
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="flex justify-end gap-2">
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => void toggleIndexerEnabled(indexer)}
                        disabled={mutatingIndexerId === indexer.id}
                        className={
                          indexer.isEnabled
                            ? "border-red-700/70 bg-red-900/60 text-red-200 hover:bg-red-900/80 hover:text-red-100"
                            : "border-emerald-300/70 dark:border-emerald-700/70 bg-emerald-100 dark:bg-emerald-900/60 text-emerald-800 dark:text-emerald-100 hover:bg-emerald-200 dark:hover:bg-emerald-800/80"
                        }
                      >
                        <Power className="mr-1 h-3.5 w-3.5" />
                        {indexer.isEnabled ? t("label.disabled") : t("label.enabled")}
                      </Button>
                      <Button
                        size="sm"
                        variant="secondary"
                        onClick={() => editIndexer(indexer)}
                      >
                        <Edit className="mr-1 h-3.5 w-3.5" />
                        {t("settings.indexerEdit")}
                      </Button>
                      <Button
                        size="sm"
                        variant="destructive"
                        onClick={() => void deleteIndexer(indexer)}
                        disabled={mutatingIndexerId === indexer.id}
                      >
                        <Trash2 className="mr-1 h-3.5 w-3.5" />
                        {mutatingIndexerId === indexer.id ? t("label.deleting") : t("settings.indexerDelete")}
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
              {settingsIndexers.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={8} className="text-muted-foreground">
                    {t("settings.noIndexersFound")}
                  </TableCell>
                </TableRow>
              ) : null}
            </TableBody>
          </Table>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">
            {editingIndexerId ? t("settings.indexerUpdate") : t("settings.indexerCreate")}
          </CardTitle>
        </CardHeader>
        <CardContent>
          <form className="space-y-3" onSubmit={submitIndexer}>
            <div className="grid gap-3 md:grid-cols-3">
              <label>
                <Label className="mb-2 block">{t("settings.indexerName")}</Label>
                <Input
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
              <label>
                <Label className="mb-2 block">{t("form.providerTypePlaceholder")}</Label>
                <Select
                  value={normalizedProviderType || "nzbgeek"}
                  onValueChange={(v) =>
                    setIndexerDraft((prev: IndexerDraft) => ({
                      ...prev,
                      providerType: v,
                    }))
                  }
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {providerTypeOptions.map((opt) => (
                      <SelectItem key={opt.value} value={opt.value}>{opt.label}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </label>
              <label>
                <Label className="mb-2 block">{t("form.baseUrlPlaceholder")}</Label>
                <Input
                  value={indexerDraft.baseUrl}
                  onChange={(event) =>
                    setIndexerDraft((prev: IndexerDraft) => ({
                      ...prev,
                      baseUrl: event.target.value,
                    }))
                  }
                  required
                  placeholder={t("form.baseUrlPlaceholderValue")}
                />
              </label>
              <label>
                <Label className="mb-2 block">{t("settings.indexerApi")}</Label>
                <Input
                  value={indexerDraft.apiKey}
                  onChange={(event) =>
                    setIndexerDraft((prev: IndexerDraft) => ({
                      ...prev,
                      apiKey: event.target.value,
                    }))
                  }
                  placeholder={t("form.apiKeyInputPlaceholder")}
                  type="password"
                />
              </label>
            </div>

            {selectedProviderFields.length > 0 ? (
              <div className="space-y-3">
                <Label className="text-sm font-medium">{t("settings.indexerConfig")}</Label>
                <div className="grid gap-3 md:grid-cols-3">
                  {selectedProviderFields
                    .filter((f) => f.fieldType !== "bool")
                    .map((field) => (
                      <DynamicConfigField
                        key={field.key}
                        field={field}
                        value={indexerDraft.configValues[field.key] ?? field.defaultValue ?? ""}
                        onChange={handleConfigValueChange}
                      />
                    ))}
                </div>
                {selectedProviderFields.some((f) => f.fieldType === "bool") ? (
                  <div className="flex items-center gap-6">
                    {selectedProviderFields
                      .filter((f) => f.fieldType === "bool")
                      .map((field) => (
                        <DynamicConfigField
                          key={field.key}
                          field={field}
                          value={indexerDraft.configValues[field.key] ?? field.defaultValue ?? "false"}
                          onChange={handleConfigValueChange}
                        />
                      ))}
                  </div>
                ) : null}
              </div>
            ) : null}

            <div className="flex items-center gap-6">
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={indexerDraft.enableInteractiveSearch}
                  onChange={(event) =>
                    setIndexerDraft((prev: IndexerDraft) => ({
                      ...prev,
                      enableInteractiveSearch: event.target.checked,
                    }))
                  }
                  className="accent-primary"
                />
                <span className="text-sm">{t("settings.indexerInteractiveSearch")}</span>
              </label>
              <label className="flex items-center gap-2">
                <input
                  type="checkbox"
                  checked={indexerDraft.enableAutoSearch}
                  onChange={(event) =>
                    setIndexerDraft((prev: IndexerDraft) => ({
                      ...prev,
                      enableAutoSearch: event.target.checked,
                    }))
                  }
                  className="accent-primary"
                />
                <span className="text-sm">{t("settings.indexerAutoSearch")}</span>
              </label>
            </div>
            <div className="flex gap-2">
              <Button type="submit" disabled={mutatingIndexerId === "new"}>
                {mutatingIndexerId === "new"
                  ? t("label.saving")
                  : editingIndexerId
                    ? t("settings.indexerUpdate")
                    : t("settings.indexerCreate")}
              </Button>
              <Button type="button" variant="secondary" onClick={resetIndexerDraft}>
                {t("label.cancel")}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
