import { Check, Loader2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { DownloadClientDraft } from "@/lib/types/download-clients";
import { SUPPORTED_DOWNLOAD_CLIENT_TYPES } from "@/lib/constants/download-clients";

interface SetupDownloadClientViewProps {
  t: (key: string) => string;
  draft: DownloadClientDraft;
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
  const showApiKey = draft.clientType === "sabnzbd";
  const showCredentials = draft.clientType === "nzbget" || draft.clientType === "qbittorrent";
  const canTest = draft.name.trim().length > 0 && draft.host.trim().length > 0;
  const canProceed = saved;

  return (
    <div className="flex flex-col gap-6">
      <div className="text-center">
        <h2 className="text-xl font-semibold">{t("setup.downloadClientTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("setup.downloadClientDescription")}</p>
      </div>
      <div className="mx-auto flex w-full max-w-md flex-col gap-4">
        <div className="space-y-2">
          <Label htmlFor="dc-name">{t("settings.name")}</Label>
          <Input
            id="dc-name"
            value={draft.name}
            onChange={(e) => onDraftChange({ name: e.target.value })}
            placeholder="My Download Client"
          />
        </div>
        <div className="space-y-2">
          <Label>{t("settings.downloadClientType")}</Label>
          <Select value={draft.clientType} onValueChange={(v) => onDraftChange({ clientType: v })}>
            <SelectTrigger><SelectValue /></SelectTrigger>
            <SelectContent>
              {SUPPORTED_DOWNLOAD_CLIENT_TYPES.map((type) => (
                <SelectItem key={type} value={type}>
                  {type}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="grid grid-cols-[1fr_auto] gap-2">
          <div className="space-y-2">
            <Label htmlFor="dc-host">{t("settings.host")}</Label>
            <Input
              id="dc-host"
              value={draft.host}
              onChange={(e) => onDraftChange({ host: e.target.value })}
              placeholder="192.168.1.100"
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="dc-port">{t("settings.port")}</Label>
            <Input
              id="dc-port"
              className="w-24"
              value={draft.port}
              onChange={(e) => onDraftChange({ port: e.target.value })}
              placeholder="8080"
            />
          </div>
        </div>
        <div className="flex items-center gap-2">
          <Checkbox
            id="dc-ssl"
            checked={draft.useSsl}
            onCheckedChange={(checked) => onDraftChange({ useSsl: checked === true })}
          />
          <Label htmlFor="dc-ssl" className="text-sm">SSL</Label>
        </div>
        {showApiKey && (
          <div className="space-y-2">
            <Label htmlFor="dc-apikey">{t("settings.apiKey")}</Label>
            <Input
              id="dc-apikey"
              type="password"
              value={draft.apiKey}
              onChange={(e) => onDraftChange({ apiKey: e.target.value })}
            />
          </div>
        )}
        {showCredentials && (
          <>
            <div className="space-y-2">
              <Label htmlFor="dc-username">{t("settings.username")}</Label>
              <Input
                id="dc-username"
                value={draft.username}
                onChange={(e) => onDraftChange({ username: e.target.value })}
              />
            </div>
            <div className="space-y-2">
              <Label htmlFor="dc-password">{t("settings.password")}</Label>
              <Input
                id="dc-password"
                type="password"
                value={draft.password}
                onChange={(e) => onDraftChange({ password: e.target.value })}
              />
            </div>
          </>
        )}
        <div className="flex items-center gap-3">
          <Button
            variant="outline"
            onClick={onTestConnection}
            disabled={!canTest || testing || saving}
          >
            {testing ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : null}
            {t("setup.testConnection")}
          </Button>
          {testResult === "success" && (
            <span className="flex items-center gap-1 text-sm text-emerald-500">
              <Check className="h-4 w-4" /> {t("setup.connectionSuccess")}
            </span>
          )}
          {testResult === "failed" && (
            <span className="flex items-center gap-1 text-sm text-destructive">
              <X className="h-4 w-4" /> {t("setup.connectionFailed")}
            </span>
          )}
        </div>
        {error && <p className="text-sm text-destructive">{error}</p>}
        {saved && (
          <p className="text-sm text-emerald-500">{t("setup.saved")}</p>
        )}
      </div>
      <div className="flex items-center justify-between pt-2">
        <Button variant="ghost" onClick={onBack}>{t("setup.back")}</Button>
        <div className="flex items-center gap-3">
          {onSkip && (
            <button type="button" onClick={onSkip} className="text-sm text-muted-foreground underline-offset-4 hover:underline">
              {t("setup.skip")}
            </button>
          )}
          <Button onClick={onNext} disabled={!canProceed || saving}>
            {saving ? t("setup.saving") : t("setup.next")}
          </Button>
        </div>
      </div>
    </div>
  );
}
