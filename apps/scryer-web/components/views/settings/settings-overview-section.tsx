import { Loader2, Shield } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useTranslate } from "@/lib/context/translate-context";
import type { LocaleCode, LanguageOption } from "@/lib/i18n";

type SettingsOverviewSectionProps = {
  availableLanguages: LanguageOption[];
  selectedLanguage: LanguageOption | null;
  uiLanguage: LocaleCode;
  onSelectLanguage: (code: string) => void;
  tlsCertPath?: string;
  setTlsCertPath?: (value: string) => void;
  tlsKeyPath?: string;
  setTlsKeyPath?: (value: string) => void;
  tlsSaving?: boolean;
  onTlsSave?: () => void;
};

export function SettingsOverviewSection({
  availableLanguages,
  uiLanguage,
  onSelectLanguage,
  tlsCertPath,
  setTlsCertPath,
  tlsKeyPath,
  setTlsKeyPath,
  tlsSaving,
  onTlsSave,
}: SettingsOverviewSectionProps) {
  const t = useTranslate();
  return (
    <div className="space-y-6 text-sm">
      <div>
        <p>{t("settings.generalText")}</p>
        <p>{t("settings.generalPlaceholder")}</p>
      </div>

      <div>
        <label className="mb-2 block text-xs font-medium uppercase tracking-wide text-muted-foreground">
          {t("label.language")}
        </label>
        <Select value={uiLanguage} onValueChange={onSelectLanguage}>
          <SelectTrigger className="w-56">
            <SelectValue placeholder={t("label.language")} />
          </SelectTrigger>
          <SelectContent>
            {availableLanguages.map((language) => (
              <SelectItem key={language.code} value={language.code}>
                {language.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {setTlsCertPath && setTlsKeyPath && onTlsSave ? (
        <div className="space-y-4 border-t border-border pt-6">
          <div className="flex items-center gap-2">
            <Shield className="h-4 w-4 text-muted-foreground" />
            <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
              {t("settings.tlsTitle")}
            </span>
          </div>
          <p className="text-xs text-muted-foreground">
            {t("settings.tlsRestartNote")}
          </p>
          <div className="space-y-3">
            <div>
              <Label htmlFor="tls-cert-path" className="mb-1 block text-xs">
                {t("settings.tlsCertPathLabel")}
              </Label>
              <Input
                id="tls-cert-path"
                value={tlsCertPath ?? ""}
                onChange={(event) => setTlsCertPath(event.target.value)}
                placeholder={t("settings.tlsCertPathPlaceholder")}
                className="max-w-md font-mono text-xs"
              />
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.tlsCertPathHelp")}
              </p>
            </div>
            <div>
              <Label htmlFor="tls-key-path" className="mb-1 block text-xs">
                {t("settings.tlsKeyPathLabel")}
              </Label>
              <Input
                id="tls-key-path"
                value={tlsKeyPath ?? ""}
                onChange={(event) => setTlsKeyPath(event.target.value)}
                placeholder={t("settings.tlsKeyPathPlaceholder")}
                className="max-w-md font-mono text-xs"
              />
              <p className="mt-1 text-xs text-muted-foreground">
                {t("settings.tlsKeyPathHelp")}
              </p>
            </div>
            <Button
              type="button"
              variant="secondary"
              className="h-9 gap-2 px-4"
              onClick={onTlsSave}
              disabled={tlsSaving}
            >
              {tlsSaving ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : null}
              <span>{t("label.save")}</span>
            </Button>
          </div>
        </div>
      ) : null}
    </div>
  );
}
