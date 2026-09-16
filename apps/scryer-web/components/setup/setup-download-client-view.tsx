import { Check, Download, X } from "lucide-react";
import { DownloadClientConfigField } from "@/components/common/download-client-config-field";
import { DownloadClientRemotePathMappingsField } from "@/components/common/download-client-remote-path-mappings-field";
import { PluginVisualLabel } from "@/components/common/plugin-visual";
import { Button } from "@/components/ui/button";
import {
  SetupBackButton,
  SetupPanel,
  SetupPrimaryButton,
  SetupStepHeader,
} from "./setup-chrome";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { ConfigFieldDef } from "@/lib/types";
import type { DownloadClientDraft, DownloadClientTypeOption } from "@/lib/types/download-clients";
import {
  buildWeaverApiKeyUrl,
  downloadClientConfigFieldValue,
  FIXED_DOWNLOAD_CLIENT_CONFIG_FIELD_KEYS,
} from "@/lib/utils/download-clients";
import type { LocalPathStyle } from "@/lib/utils/local-path-style";
import * as React from "react";
import { LoadingMark } from "@/components/common/loading-mark";

function DownloadClientTypeOptionContent({
  typeValue,
  label,
}: {
  typeValue: string;
  label: string;
}) {
  return (
    <PluginVisualLabel
      providerType={typeValue}
      pluginType="download_client"
      label={label}
    />
  );
}

interface SetupDownloadClientViewProps {
  t: (key: string) => string;
  draft: DownloadClientDraft;
  downloadClientTypeOptions: DownloadClientTypeOption[];
  configFields: ConfigFieldDef[];
  localPathStyle: LocalPathStyle | undefined;
  onDraftChange: (updates: Partial<DownloadClientDraft>) => void;
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

export function SetupDownloadClientView({
  t,
  draft,
  downloadClientTypeOptions,
  configFields,
  localPathStyle,
  onDraftChange,
  onTestConnection,
  onNext,
  onBack,
  onSkip,
  testing,
  testResult,
  saving,
  saved,
  error,
}: SetupDownloadClientViewProps) {
  const [areRemotePathMappingsValid, setAreRemotePathMappingsValid] = React.useState(true);
  const [isFilesystemPathMappingOpen, setIsFilesystemPathMappingOpen] = React.useState(() =>
    draft.remotePathMappings.trim().length > 0,
  );
  const normalizedClientType = draft.clientType.trim().toLowerCase();
  const dynamicConfigFields = configFields.filter(
    (field) => !FIXED_DOWNLOAD_CLIENT_CONFIG_FIELD_KEYS.has(field.key),
  );
  const selectedFieldKeys = new Set(configFields.map((field) => field.key));
  const hasDescriptorApiKeyField =
    selectedFieldKeys.has("api_key") || selectedFieldKeys.has("apiKey");
  const hasDescriptorCredentialFields =
    selectedFieldKeys.has("username") || selectedFieldKeys.has("password");
  const showApiKey =
    !hasDescriptorApiKeyField &&
    (normalizedClientType === "sabnzbd" || normalizedClientType === "weaver");
  const showCredentials =
    !hasDescriptorCredentialFields &&
    (normalizedClientType === "nzbget" ||
      normalizedClientType === "qbittorrent" ||
      normalizedClientType === "sabnzbd");
  const showSabAlternativeAuth = normalizedClientType === "sabnzbd";
  const showDecypharrFilesystemHelp =
    normalizedClientType === "sabnzbd" || normalizedClientType === "qbittorrent";
  const weaverApiKeyUrl = normalizedClientType === "weaver" ? buildWeaverApiKeyUrl(draft) : "";
  const selectedDownloadClientLabel =
    downloadClientTypeOptions.find((option) => option.value === normalizedClientType)?.label ??
    (draft.clientType.trim() || "Download client");
  React.useEffect(() => {
    if (draft.remotePathMappings.trim().length > 0 || !areRemotePathMappingsValid) {
      setIsFilesystemPathMappingOpen(true);
    } else {
      setIsFilesystemPathMappingOpen(false);
    }
  }, [areRemotePathMappingsValid, draft.remotePathMappings]);

  const canTest =
    draft.name.trim().length > 0 &&
    draft.host.trim().length > 0 &&
    areRemotePathMappingsValid;
  const canProceed = saved && areRemotePathMappingsValid;

  return (
    <SetupPanel id="setup-download-client-view" className="flex flex-col gap-6">
      <SetupStepHeader
        icon={Download}
        title={t("setup.downloadClientTitle")}
        subtitle={t("setup.downloadClientDescription")}
      />
      <div className="mx-auto grid w-full max-w-4xl gap-x-6 gap-y-4 md:grid-cols-2">
        <div className="space-y-2 md:col-span-2">
          <Label htmlFor="setup-download-client-name">{t("label.name")}</Label>
          <Input
            id="setup-download-client-name"
            value={draft.name}
            onChange={(e) => onDraftChange({ name: e.target.value })}
            placeholder="My Download Client"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="setup-download-client-type">{t("label.type")}</Label>
          <Select value={draft.clientType} onValueChange={(v) => onDraftChange({ clientType: v })}>
            <SelectTrigger id="setup-download-client-type" className="w-full">
              <SelectValue aria-label={selectedDownloadClientLabel}>
                <DownloadClientTypeOptionContent
                  typeValue={draft.clientType}
                  label={selectedDownloadClientLabel}
                />
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              {downloadClientTypeOptions.map((option) => (
                <SelectItem
                  key={option.value}
                  value={option.value}
                  textValue={option.label}
                >
                  <DownloadClientTypeOptionContent
                    typeValue={option.value}
                    label={option.label}
                  />
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="grid grid-cols-[minmax(0,1fr)_6rem] gap-2 md:col-span-2 md:grid-cols-[minmax(0,1fr)_8rem]">
          <div className="space-y-2">
            <Label htmlFor="setup-download-client-host">{t("settings.host")}</Label>
            <Input
              id="setup-download-client-host"
              value={draft.host}
              onChange={(e) => onDraftChange({ host: e.target.value })}
              placeholder="192.168.1.100"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="setup-download-client-port">{t("settings.port")}</Label>
            <Input
              id="setup-download-client-port"
              {...integerInputProps}
              className="w-24"
              value={draft.port}
              onChange={(e) => onDraftChange({ port: sanitizeDigits(e.target.value) })}
              placeholder="8080"
            />
          </div>
        </div>
        <div className="flex items-center gap-2 md:col-span-2">
          <Checkbox
            id="setup-download-client-ssl"
            checked={draft.useSsl}
            onCheckedChange={(checked) => onDraftChange({ useSsl: checked === true })}
          />
          <Label htmlFor="setup-download-client-ssl" className="text-sm">SSL</Label>
        </div>
        {showApiKey && (
          <div className="space-y-2 md:col-span-2">
            <Label htmlFor="setup-download-client-api-key">{t("settings.apiKey")}</Label>
            <Input
              id="setup-download-client-api-key"
              type="password"
              ignorePasswordManagers
              value={draft.apiKey}
              onChange={(e) => onDraftChange({ apiKey: e.target.value })}
            />
            {normalizedClientType === "weaver" ? (
              <p className="text-xs text-muted-foreground">
                Create an integration API key in Weaver:{" "}
                {weaverApiKeyUrl ? (
                  <a
                    href={weaverApiKeyUrl}
                    target="_blank"
                    rel="noreferrer"
                    className="underline underline-offset-4 hover:text-foreground"
                  >
                    open Weaver security settings
                  </a>
                ) : (
                  <span>finish the Weaver URL above to generate the link.</span>
                )}
              </p>
            ) : normalizedClientType === "sabnzbd" ? (
              <div className="space-y-2 text-xs text-muted-foreground">
                <p>{t("settings.downloadClientSabnzbdAuthHelp")}</p>
                <p>{t("settings.downloadClientSabnzbdNzbdavHelp")}</p>
              </div>
            ) : null}
          </div>
        )}
        {showCredentials && (
          <>
            <div className="space-y-2">
              <Label htmlFor="setup-download-client-username">
                {t("settings.username")}
                {showSabAlternativeAuth ? " (optional)" : ""}
              </Label>
              <Input
                id="setup-download-client-username"
                value={draft.username}
                onChange={(e) => onDraftChange({ username: e.target.value })}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="setup-download-client-password">
                {t("settings.password")}
                {showSabAlternativeAuth ? " (optional)" : ""}
              </Label>
              <Input
                id="setup-download-client-password"
                type="password"
                ignorePasswordManagers
                value={draft.password}
                onChange={(e) => onDraftChange({ password: e.target.value })}
              />
            </div>
            {normalizedClientType === "qbittorrent" ? (
              <p className="text-xs text-muted-foreground md:col-span-2">
                {t("settings.downloadClientQbittorrentDecypharrHelp")}
              </p>
            ) : null}
          </>
        )}
        {dynamicConfigFields.map((field) => (
          <div
            key={field.key}
            className={field.fieldType === "MULTILINE" ? "md:col-span-2" : undefined}
          >
            <DownloadClientConfigField
              field={field}
              value={downloadClientConfigFieldValue(draft, field)}
              idPrefix="setup-download-client-field"
              onChange={(key, value) =>
                onDraftChange({
                  configValues: {
                    ...draft.configValues,
                    [key]: value,
                  },
                })
              }
            />
          </div>
        ))}
        {showDecypharrFilesystemHelp ? (
          <p className="text-xs text-muted-foreground md:col-span-2">
            {t("settings.downloadClientDecypharrFilesystemHelp")}
          </p>
        ) : null}
        <details
          id="setup-download-client-filesystem-path-mapping"
          className="rounded-xl border border-border bg-card p-3 md:col-span-2"
          open={isFilesystemPathMappingOpen}
          onToggle={(event) =>
            setIsFilesystemPathMappingOpen(event.currentTarget.open)
          }
        >
          <summary
            id="setup-download-client-filesystem-path-mapping-toggle"
            className="cursor-pointer select-none text-sm font-medium text-card-foreground"
          >
            {t("settings.downloadClientFilesystemPathMapping")}
          </summary>
          <div className="mt-3 space-y-3">
            <p className="text-xs text-muted-foreground">
              {t("settings.downloadClientFilesystemPathMappingHelp")}
            </p>
            <DownloadClientRemotePathMappingsField
              fieldKey="remote_path_mappings"
              label={t("settings.downloadClientRemotePathMappings")}
              value={draft.remotePathMappings}
              helpText={t("settings.downloadClientRemotePathMappingsHelp")}
              localPathStyle={localPathStyle}
              translate={t}
              onValidityChange={setAreRemotePathMappingsValid}
              onChange={(_, value) => onDraftChange({ remotePathMappings: value })}
            />
          </div>
        </details>
        <div className="flex items-center gap-3 md:col-span-2">
          <Button
            id="setup-download-client-test-connection"
            variant="outline"
            onClick={onTestConnection}
            disabled={!canTest || testing || saving}
          >
            {testing ? (
              <LoadingMark className="mr-2 h-4 w-4" />
            ) : null}
            {t("label.testConnection")}
          </Button>
          {testResult === "success" && (
            <span
              id="setup-download-client-test-result-success"
              className="flex items-center gap-1 text-sm text-[var(--scry-success-text-soft)]"
            >
              <Check className="h-4 w-4" /> {t("setup.connectionSuccess")}
            </span>
          )}
          {testResult === "failed" && (
            <span
              id="setup-download-client-test-result-failed"
              className="flex items-center gap-1 text-sm text-destructive"
            >
              <X className="h-4 w-4" /> {t("setup.connectionFailed")}
            </span>
          )}
        </div>
        {error && <p id="setup-download-client-error" className="text-sm text-destructive md:col-span-2">{error}</p>}
        {saved && (
          <p id="setup-download-client-saved" className="text-sm text-[var(--scry-success-text-soft)] md:col-span-2">{t("setup.saved")}</p>
        )}
      </div>
      <div className="flex items-center justify-between pt-2">
        <SetupBackButton id="setup-download-client-back" onClick={onBack}>
          {t("setup.back")}
        </SetupBackButton>
        <div className="flex items-center gap-3">
          {onSkip && (
            <Button id="setup-download-client-skip" type="button" variant="link" onClick={onSkip}>
              {t("setup.skip")}
            </Button>
          )}
          <SetupPrimaryButton id="setup-download-client-next" onClick={onNext} disabled={!canProceed || saving}>
            {saving ? t("label.saving") : t("setup.next")}
          </SetupPrimaryButton>
        </div>
      </div>
    </SetupPanel>
  );
}
