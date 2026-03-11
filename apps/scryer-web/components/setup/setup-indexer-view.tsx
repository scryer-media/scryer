import { Check, Loader2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";

interface ProviderOption {
  value: string;
  label: string;
  defaultBaseUrl?: string;
}

interface SetupIndexerViewProps {
  t: (key: string) => string;
  name: string;
  providerType: string;
  baseUrl: string;
  apiKey: string;
  providerOptions: ProviderOption[];
  onNameChange: (value: string) => void;
  onProviderTypeChange: (value: string) => void;
  onBaseUrlChange: (value: string) => void;
  onApiKeyChange: (value: string) => void;
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

export function SetupIndexerView({
  t,
  name,
  providerType,
  baseUrl,
  apiKey,
  providerOptions,
  onNameChange,
  onProviderTypeChange,
  onBaseUrlChange,
  onApiKeyChange,
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
  const hasDefaultUrl = !!selectedProvider?.defaultBaseUrl;
  const canTest = name.trim().length > 0 && providerType.length > 0;
  const canProceed = saved;

  return (
    <div className="flex flex-col gap-6">
      <div className="text-center">
        <h2 className="text-xl font-semibold">{t("setup.indexerTitle")}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{t("setup.indexerDescription")}</p>
      </div>
      <div className="mx-auto flex w-full max-w-md flex-col gap-4">
        <div className="space-y-2">
          <Label htmlFor="idx-name">{t("label.name")}</Label>
          <Input
            id="idx-name"
            value={name}
            onChange={(e) => onNameChange(e.target.value)}
            placeholder="My Indexer"
          />
        </div>
        <div className="space-y-2">
          <Label>{t("settings.indexerProvider")}</Label>
          <Select value={providerType} onValueChange={onProviderTypeChange}>
            <SelectTrigger><SelectValue placeholder="Select provider" /></SelectTrigger>
            <SelectContent>
              {providerOptions.map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>
                  {opt.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        {!hasDefaultUrl && (
          <div className="space-y-2">
            <Label htmlFor="idx-url">{t("settings.baseUrl")}</Label>
            <Input
              id="idx-url"
              value={baseUrl}
              onChange={(e) => onBaseUrlChange(e.target.value)}
              placeholder="https://api.example.com"
            />
          </div>
        )}
        <div className="space-y-2">
          <Label htmlFor="idx-apikey">{t("settings.apiKey")}</Label>
          <Input
            id="idx-apikey"
            type="password"
            value={apiKey}
            onChange={(e) => onApiKeyChange(e.target.value)}
          />
        </div>
        <div className="flex items-center gap-3">
          <Button
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
            {saving ? t("label.saving") : t("setup.next")}
          </Button>
        </div>
      </div>
    </div>
  );
}
