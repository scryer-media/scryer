import * as React from "react";
import { AlertTriangle, ChevronDown, Loader2, Rocket, ShieldPlus, Trash2, Upload } from "lucide-react";
import { Link } from "react-router";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Input, integerInputProps, sanitizeDigits } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SingleSelectField } from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";
import {
  TrustedCertificateUploadError,
  bundlePemFromTrustedCertificateEntries,
  mergeTrustedCertificateEntries,
  readTrustedCertificateEntriesFromFiles,
} from "@/lib/utils/certificates";
import {
  parseUiDateTimeFormat,
  parseVerificationDepth,
} from "@/lib/utils/settings-mutation-inputs";
import type {
  GeneralSettings,
  GeneralSettingsUpdate,
  TrustedCertificateEntry,
  UiDateTimeFormat,
  VerificationDepth,
} from "@/lib/types/settings";

type SettingsOverviewSectionProps = {
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
  dateTimeFormat: UiDateTimeFormat;
  dateTimeFormatLoading: boolean;
  dateTimeFormatSaving: boolean;
  onDateTimeFormatChange: (format: UiDateTimeFormat) => void;
  generalSettings: GeneralSettings;
  onGeneralSettingsChange: (settings: GeneralSettings) => void;
  generalLoading: boolean;
  generalSaving: boolean;
  imageCacheClearing: boolean;
  onClearImageCache: () => void;
  onGeneralSettingsCommit: (update: GeneralSettingsUpdate) => void;
  verificationDepth: VerificationDepth;
  verificationLoading: boolean;
  verificationSaving: boolean;
  onVerificationDepthChange: (depth: VerificationDepth) => void;
};

export function SettingsOverviewSection({
  availableLanguages,
  uiLanguage,
  onSelectLanguage,
  dateTimeFormat,
  dateTimeFormatLoading,
  dateTimeFormatSaving,
  onDateTimeFormatChange,
  generalSettings,
  onGeneralSettingsChange,
  generalLoading,
  generalSaving,
  imageCacheClearing,
  onClearImageCache,
  onGeneralSettingsCommit,
  verificationDepth,
  verificationLoading,
  verificationSaving,
  onVerificationDepthChange,
}: SettingsOverviewSectionProps) {
  const t = useTranslate();
  const [advancedTrustOpen, setAdvancedTrustOpen] = React.useState(false);
  const [trustedCertUploadError, setTrustedCertUploadError] = React.useState<string | null>(null);
  const [trustedCertUploading, setTrustedCertUploading] = React.useState(false);
  const trustedCertInputRef = React.useRef<HTMLInputElement | null>(null);
  const updateGeneralSettings = React.useCallback(
    (patch: Partial<GeneralSettings>) =>
      onGeneralSettingsChange({ ...generalSettings, ...patch }),
    [generalSettings, onGeneralSettingsChange],
  );
  const applyTrustedCertificateEntries = React.useCallback(
    (entries: TrustedCertificateEntry[]) => {
      const pluginHttpCaBundlePem = bundlePemFromTrustedCertificateEntries(entries);
      updateGeneralSettings({
        pluginHttpTrustedCertificates: entries,
        pluginHttpCaBundlePem,
      });
      onGeneralSettingsCommit({ pluginHttpCaBundlePem });
    },
    [onGeneralSettingsCommit, updateGeneralSettings],
  );
  const mapTrustedCertUploadError = React.useCallback(
    (error: unknown) => {
      if (error instanceof TrustedCertificateUploadError) {
        switch (error.code) {
          case "pem_bundle_missing_certificate":
            return t("settings.pluginHttpTrustUploadMissingCertificate");
          case "pem_bundle_trailing_text":
            return t("settings.pluginHttpTrustUploadTrailingText");
          case "pem_bundle_invalid_certificate":
            return t("settings.pluginHttpTrustUploadInvalidCertificate");
        }
      }
      return t("settings.pluginHttpTrustUploadReadError");
    },
    [t],
  );
  const handleTrustedCertificateUpload = React.useCallback(
    async (event: React.ChangeEvent<HTMLInputElement>) => {
      const files = event.target.files ? [...event.target.files] : [];
      event.target.value = "";
      if (files.length === 0) {
        return;
      }

      setTrustedCertUploading(true);
      setTrustedCertUploadError(null);
      try {
        const incoming = await readTrustedCertificateEntriesFromFiles(files);
        const merged = mergeTrustedCertificateEntries(
          generalSettings.pluginHttpTrustedCertificates,
          incoming,
        );
        applyTrustedCertificateEntries(merged);
      } catch (error) {
        setTrustedCertUploadError(mapTrustedCertUploadError(error));
      } finally {
        setTrustedCertUploading(false);
      }
    },
    [
      applyTrustedCertificateEntries,
      generalSettings.pluginHttpTrustedCertificates,
      mapTrustedCertUploadError,
    ],
  );
  const handleRemoveTrustedCertificate = React.useCallback(
    (fingerprintSha256: string) => {
      setTrustedCertUploadError(null);
      applyTrustedCertificateEntries(
        generalSettings.pluginHttpTrustedCertificates.filter(
          (entry) => entry.fingerprintSha256 !== fingerprintSha256,
        ),
      );
    },
    [applyTrustedCertificateEntries, generalSettings.pluginHttpTrustedCertificates],
  );
  const handleClearTrustedCertificates = React.useCallback(() => {
    setTrustedCertUploadError(null);
    applyTrustedCertificateEntries([]);
  }, [applyTrustedCertificateEntries]);

  return (
    <div className="space-y-6 text-sm">
      <div>
        <p>{t("settings.generalText")}</p>
        <p>{t("settings.generalPlaceholder")}</p>
      </div>

      <SingleSelectField
        label={t("label.language")}
        value={uiLanguage}
        onValueChange={onSelectLanguage}
        placeholder={t("label.language")}
        triggerClassName="w-56"
        options={availableLanguages.map((language) => ({
          value: language.code,
          label: language.label,
        }))}
      />

      <SingleSelectField
        label={t("settings.dateTimeFormatLabel")}
        value={dateTimeFormat}
        disabled={dateTimeFormatLoading || dateTimeFormatSaving}
        onValueChange={(value) => {
          const format = parseUiDateTimeFormat(value);
          if (format) {
            onDateTimeFormatChange(format);
          }
        }}
        placeholder={t("settings.dateTimeFormatLabel")}
        description={t("settings.dateTimeFormatHelp")}
        triggerClassName="w-64"
        options={[
          {
            value: "LOCALE",
            label: t("settings.dateTimeFormatLocale"),
          },
          {
            value: "ISO24H",
            label: t("settings.dateTimeFormatIso24h"),
          },
        ]}
      />

      <div className="space-y-4 border-t border-border pt-6">
        <div className="space-y-1">
          <h3 className="text-sm font-semibold">{t("settings.featuresHeader")}</h3>
        </div>

        {generalLoading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("label.loading")}
          </div>
        ) : (
          <>
            <div className="space-y-1">
              <div className="flex items-center gap-3">
                <Label>{t("settings.experimentalFeaturesLabel")}</Label>
                <SettingsToggleSwitch
                  checked={generalSettings.experimentalFeaturesEnabled}
                  ariaLabel={t("settings.experimentalFeaturesLabel")}
                  disabled={generalSaving}
                  onChange={(nextValue) => {
                    updateGeneralSettings({ experimentalFeaturesEnabled: nextValue });
                    onGeneralSettingsCommit({ experimentalFeaturesEnabled: nextValue });
                  }}
                />
              </div>
              <p className="text-muted-foreground">
                {t("settings.experimentalFeaturesHelp")}
              </p>
            </div>

            <div className="space-y-1">
              <div className="flex items-center gap-3">
                <Label>{t("settings.personalizedDiscoveryLabel")}</Label>
                <SettingsToggleSwitch
                  checked={generalSettings.personalizedDiscoveryEnabled}
                  ariaLabel={t("settings.personalizedDiscoveryLabel")}
                  disabled={generalSaving}
                  onChange={(nextValue) => {
                    updateGeneralSettings({ personalizedDiscoveryEnabled: nextValue });
                    onGeneralSettingsCommit({ personalizedDiscoveryEnabled: nextValue });
                  }}
                />
              </div>
              <p className="text-muted-foreground">
                {t("settings.personalizedDiscoveryHelp")}
              </p>
            </div>

            <div className="space-y-1">
              <div className="flex items-center gap-3">
                <Label>{t("settings.srrdbFilenameRecoveryLabel")}</Label>
                <SettingsToggleSwitch
                  checked={generalSettings.srrdbFilenameRecoveryEnabled}
                  ariaLabel={t("settings.srrdbFilenameRecoveryLabel")}
                  disabled={generalSaving}
                  onChange={(nextValue) => {
                    updateGeneralSettings({ srrdbFilenameRecoveryEnabled: nextValue });
                    onGeneralSettingsCommit({ srrdbFilenameRecoveryEnabled: nextValue });
                  }}
                />
              </div>
              <p className="text-muted-foreground">
                {t("settings.srrdbFilenameRecoveryHelp")}
              </p>
            </div>
          </>
        )}
      </div>

      <div className="space-y-4 border-t border-border pt-6">
        <div className="space-y-1">
          <h3 className="text-sm font-semibold">{t("settings.historyRetentionTitle")}</h3>
          <p className="text-muted-foreground">
            {t("settings.historyRetentionHelp")}
          </p>
          <p className="text-muted-foreground">
            {t("settings.historyRetentionExternalHelp")}
          </p>
        </div>

        {generalLoading ? (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Loader2 className="h-4 w-4 animate-spin" />
            {t("label.loading")}
          </div>
        ) : (
          <>
            <div className="flex items-center gap-3">
              <Label>{t("settings.keepHistoryForever")}</Label>
              <SettingsToggleSwitch
                checked={generalSettings.keepHistoryForever}
                ariaLabel={t("settings.keepHistoryForever")}
                disabled={generalSaving}
                onChange={(nextValue) => {
                  updateGeneralSettings({ keepHistoryForever: nextValue });
                  onGeneralSettingsCommit({ keepHistoryForever: nextValue });
                }}
              />
            </div>

            <div className="space-y-1">
              <Label>{t("settings.historyRetentionDaysHeader")}</Label>
              <div className="relative w-36">
                <Input
                  {...integerInputProps}
                  className="pr-16"
                  disabled={generalSettings.keepHistoryForever || generalSaving}
                  value={generalSettings.historyRetentionDays}
                  onChange={(event) => {
                    const nextValue = sanitizeDigits(event.target.value);
                    updateGeneralSettings({
                      historyRetentionDays: nextValue === "" ? 0 : Number(nextValue),
                    });
                  }}
                  onBlur={() =>
                    onGeneralSettingsCommit({
                      historyRetentionDays: generalSettings.historyRetentionDays,
                    })}
                />
                <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center text-xs text-muted-foreground">
                  {t("settings.historyRetentionDaysSuffix")}
                </span>
              </div>
            </div>

            <div className="space-y-3 border-t border-border/60 pt-4">
              <h3 className="text-sm font-semibold">{t("settings.imageCacheTitle")}</h3>
              <div className="space-y-1">
                <Label>{t("settings.imageCacheMaxSizeLabel")}</Label>
                <div className="flex items-center gap-2">
                  <div className="relative w-40">
                    <Input
                      {...integerInputProps}
                      className="pr-14"
                      disabled={generalSaving}
                      value={generalSettings.imageCacheMaxSizeMb}
                      onChange={(event) => {
                        const nextValue = sanitizeDigits(event.target.value);
                        updateGeneralSettings({
                          imageCacheMaxSizeMb: nextValue === "" ? 0 : Number(nextValue),
                        });
                      }}
                      onBlur={() =>
                        onGeneralSettingsCommit({
                          imageCacheMaxSizeMb: generalSettings.imageCacheMaxSizeMb,
                        })}
                    />
                    <span className="pointer-events-none absolute inset-y-0 right-3 flex items-center text-xs text-muted-foreground">
                      {t("settings.imageCacheMbSuffix")}
                    </span>
                  </div>
                  <IconButton
                    id="settings-general-clear-image-cache"
                    label={
                      imageCacheClearing
                        ? t("settings.clearingImageCache")
                        : t("settings.clearImageCache")
                    }
                    tone="delete"
                    onClick={onClearImageCache}
                    disabled={imageCacheClearing}
                  >
                    {imageCacheClearing ? (
                      <Loader2 className="h-4 w-4 animate-spin" />
                    ) : (
                      <Trash2 className="h-4 w-4" />
                    )}
                  </IconButton>
                </div>
              </div>
              {generalSettings.imageCacheMaxSizeEnvOverrideActive ? (
                <div className="rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-3 text-xs text-muted-foreground">
                  {t("settings.imageCacheEnvOverride")}
                </div>
              ) : null}
            </div>

          </>
        )}
      </div>

      <div
        id="settings-general-verification-depth-section"
        className="space-y-4 border-t border-border pt-6"
      >
        <div className="space-y-1">
          <h3 className="text-sm font-semibold">{t("settings.verificationDepthTitle")}</h3>
          <p className="text-muted-foreground">{t("settings.verificationDepthHelp")}</p>
          <p className="text-muted-foreground">{t("settings.verificationDepthScope")}</p>
        </div>
        <SingleSelectField
          id="settings-general-verification-depth"
          label={t("settings.verificationDepthLabel")}
          value={verificationDepth}
          disabled={verificationLoading || verificationSaving}
          onValueChange={(value) => {
            const depth = parseVerificationDepth(value);
            if (depth) {
              onVerificationDepthChange(depth);
            }
          }}
          placeholder={t("settings.verificationDepthLabel")}
          description={
            verificationDepth === "QUICK"
              ? t("settings.verificationDepthQuickHelp")
              : t("settings.verificationDepthFullHelp")
          }
          triggerClassName="w-72"
          options={[
            {
              value: "FULL",
              label: t("settings.verificationDepthFull"),
            },
            {
              value: "QUICK",
              label: t("settings.verificationDepthQuick"),
            },
          ]}
        />
      </div>

      <div
        id="settings-general-plugin-http-trust-section"
        className="space-y-4 border-t border-border pt-6"
      >
        <Collapsible open={advancedTrustOpen} onOpenChange={setAdvancedTrustOpen}>
          <CollapsibleTrigger asChild>
            <Button
              id="settings-general-plugin-http-trust-toggle"
              type="button"
              variant="outline"
              className="flex w-full items-center justify-between gap-3"
            >
              <span className="flex items-center gap-2">
                <ShieldPlus className="h-4 w-4" />
                <span className="text-left">
                  {t("settings.pluginHttpTrustTitle")}
                </span>
                <Badge tone="warning" className="rounded-full text-[10px] font-semibold uppercase tracking-wide">
                  {t("settings.pluginHttpTrustAdvancedLabel")}
                </Badge>
              </span>
              <span className="flex items-center gap-2 text-xs text-muted-foreground">
                <span>
                  {generalSettings.pluginHttpTrustedCertificates.length > 0
                    ? t("settings.pluginHttpTrustStoredCount", {
                        count: generalSettings.pluginHttpTrustedCertificates.length,
                      })
                    : t("settings.pluginHttpTrustEmpty")}
                </span>
                <ChevronDown
                  className={`h-4 w-4 transition-transform ${advancedTrustOpen ? "rotate-180" : ""}`}
                />
              </span>
            </Button>
          </CollapsibleTrigger>
          <CollapsibleContent className="space-y-4 pt-4">
            <div className="rounded-lg border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] p-3 text-sm">
              <div className="flex items-start gap-2">
                <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-[var(--scry-warning-text)]" />
                <div className="space-y-1">
                  <p className="font-medium">{t("settings.pluginHttpTrustWarningTitle")}</p>
                  <p className="text-muted-foreground">
                    {t("settings.pluginHttpTrustWarningBody")}
                  </p>
                </div>
              </div>
            </div>

            <div className="space-y-2">
              <p className="text-muted-foreground">
                {t("settings.pluginHttpTrustDescription")}
              </p>
              <input
                id="settings-general-plugin-http-trust-upload-input"
                ref={trustedCertInputRef}
                type="file"
                accept=".pem,.crt,.cer,.der"
                multiple
                className="hidden"
                onChange={handleTrustedCertificateUpload}
              />
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  id="settings-general-plugin-http-trust-upload-button"
                  type="button"
                  variant="outline"
                  disabled={trustedCertUploading || generalSaving}
                  onClick={() => trustedCertInputRef.current?.click()}
                >
                  {trustedCertUploading ? (
                    <>
                      <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                      {t("label.loading")}
                    </>
                  ) : (
                    <>
                      <Upload className="mr-2 h-4 w-4" />
                      {t("settings.pluginHttpTrustUploadButton")}
                    </>
                  )}
                </Button>
                <Button
                  id="settings-general-plugin-http-trust-clear"
                  type="button"
                  variant="ghost"
                  disabled={
                    generalSaving ||
                    generalSettings.pluginHttpTrustedCertificates.length === 0
                  }
                  onClick={handleClearTrustedCertificates}
                >
                  {t("settings.pluginHttpTrustClearButton")}
                </Button>
              </div>
              {trustedCertUploadError ? (
                <p className="text-sm text-destructive">{trustedCertUploadError}</p>
              ) : null}
            </div>

            <div className="space-y-2">
              {generalSettings.pluginHttpTrustedCertificates.length === 0 ? (
                <p className="text-sm text-muted-foreground">
                  {t("settings.pluginHttpTrustEmpty")}
                </p>
              ) : (
                generalSettings.pluginHttpTrustedCertificates.map((entry) => (
                  <div
                    id={`settings-general-plugin-http-trust-entry-${entry.fingerprintSha256}`}
                    key={entry.fingerprintSha256}
                    className="flex items-center justify-between gap-3 rounded-lg border border-border/60 bg-muted/30 px-3 py-2"
                  >
                    <div className="space-y-1">
                      <p className="font-medium">
                        {t("settings.pluginHttpTrustEntryLabel")}
                      </p>
                      <code className="text-xs text-muted-foreground">
                        {entry.fingerprintSha256}
                      </code>
                    </div>
                    <IconButton
                      id={`settings-general-plugin-http-trust-remove-${entry.fingerprintSha256}`}
                      label={t("label.remove")}
                      appearance="ghost"
                      disabled={generalSaving}
                      onClick={() => handleRemoveTrustedCertificate(entry.fingerprintSha256)}
                    >
                      <Trash2 className="h-4 w-4" />
                    </IconButton>
                  </div>
                ))
              )}
            </div>
          </CollapsibleContent>
        </Collapsible>
      </div>

      <div className="border-t border-border pt-6">
        <Button asChild variant="primary" className="gap-2">
          <Link to={{ pathname: "/setup", search: "?reentry=1" }}>
            <Rocket className="h-4 w-4" />
            {t("settings.runSetupWizard")}
          </Link>
        </Button>
      </div>
    </div>
  );
}
