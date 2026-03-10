import { ArrowLeft, ExternalLink, Loader2, Tv, Film } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

interface SetupImportConnectViewProps {
  t: (key: string) => string;
  sonarrUrl: string;
  sonarrApiKey: string;
  radarrUrl: string;
  radarrApiKey: string;
  onSonarrUrlChange: (v: string) => void;
  onSonarrApiKeyChange: (v: string) => void;
  onRadarrUrlChange: (v: string) => void;
  onRadarrApiKeyChange: (v: string) => void;
  onConnect: () => void;
  onBack: () => void;
  connecting: boolean;
  error: string | null;
}

function normalizeUrl(raw: string): string {
  let url = raw.trim().replace(/\/+$/, "");
  if (!url) return url;
  if (!/^https?:\/\//i.test(url)) url = `http://${url}`;
  return url;
}

function settingsUrl(raw: string): string | null {
  const base = normalizeUrl(raw);
  if (!base) return null;
  try {
    new URL(base);
    return `${base}/settings/general`;
  } catch {
    return null;
  }
}

export function SetupImportConnectView({
  t,
  sonarrUrl,
  sonarrApiKey,
  radarrUrl,
  radarrApiKey,
  onSonarrUrlChange,
  onSonarrApiKeyChange,
  onRadarrUrlChange,
  onRadarrApiKeyChange,
  onConnect,
  onBack,
  connecting,
  error,
}: SetupImportConnectViewProps) {
  const hasSonarr = sonarrUrl.trim().length > 0 && sonarrApiKey.trim().length > 0;
  const hasRadarr = radarrUrl.trim().length > 0 && radarrApiKey.trim().length > 0;
  const canConnect = hasSonarr || hasRadarr;

  const sonarrSettingsUrl = settingsUrl(sonarrUrl);
  const radarrSettingsUrl = settingsUrl(radarrUrl);

  return (
    <div className="w-full space-y-6">
      <div className="text-center">
        <h2 className="mb-2 text-xl font-semibold">{t("setup.connectTitle")}</h2>
        <p className="text-sm text-muted-foreground">{t("setup.connectDescription")}</p>
      </div>

      <div className="grid gap-6 md:grid-cols-2">
        {/* Sonarr */}
        <div className="space-y-3 rounded-lg border border-border p-4">
          <div className="flex items-center gap-2 font-medium">
            <Tv className="h-4 w-4 text-blue-500" />
            <span>Sonarr</span>
            <span className="text-xs text-muted-foreground">(series)</span>
          </div>
          <div>
            <label className="mb-1 block text-xs text-muted-foreground">
              {t("setup.sonarrUrl")}
            </label>
            <Input
              value={sonarrUrl}
              onChange={(e) => onSonarrUrlChange(e.target.value)}
              onBlur={() => onSonarrUrlChange(normalizeUrl(sonarrUrl))}
              placeholder="http://localhost:8989"
              className="font-mono text-xs"
            />
          </div>
          <div>
            <label className="mb-1 block text-xs text-muted-foreground">
              {t("setup.sonarrApiKey")}
            </label>
            <Input
              type="password"
              value={sonarrApiKey}
              onChange={(e) => onSonarrApiKeyChange(e.target.value)}
              placeholder="API key from Settings > General"
              className="font-mono text-xs"
            />
            {sonarrSettingsUrl ? (
              <a
                href={sonarrSettingsUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="mt-1 inline-flex items-center gap-1 text-[11px] text-blue-500 hover:underline"
              >
                {t("setup.findApiKey")}
                <ExternalLink className="h-3 w-3" />
              </a>
            ) : null}
          </div>
        </div>

        {/* Radarr */}
        <div className="space-y-3 rounded-lg border border-border p-4">
          <div className="flex items-center gap-2 font-medium">
            <Film className="h-4 w-4 text-amber-500" />
            <span>Radarr</span>
            <span className="text-xs text-muted-foreground">(movies)</span>
          </div>
          <div>
            <label className="mb-1 block text-xs text-muted-foreground">
              {t("setup.radarrUrl")}
            </label>
            <Input
              value={radarrUrl}
              onChange={(e) => onRadarrUrlChange(e.target.value)}
              onBlur={() => onRadarrUrlChange(normalizeUrl(radarrUrl))}
              placeholder="http://localhost:7878"
              className="font-mono text-xs"
            />
          </div>
          <div>
            <label className="mb-1 block text-xs text-muted-foreground">
              {t("setup.radarrApiKey")}
            </label>
            <Input
              type="password"
              value={radarrApiKey}
              onChange={(e) => onRadarrApiKeyChange(e.target.value)}
              placeholder="API key from Settings > General"
              className="font-mono text-xs"
            />
            {radarrSettingsUrl ? (
              <a
                href={radarrSettingsUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="mt-1 inline-flex items-center gap-1 text-[11px] text-amber-500 hover:underline"
              >
                {t("setup.findApiKey")}
                <ExternalLink className="h-3 w-3" />
              </a>
            ) : null}
          </div>
        </div>
      </div>

      {error ? (
        <p className="text-center text-sm text-destructive">{error}</p>
      ) : null}

      {!canConnect ? (
        <p className="text-center text-xs text-muted-foreground">
          {t("setup.atLeastOneRequired")}
        </p>
      ) : null}

      <div className="flex items-center justify-between">
        <Button variant="ghost" onClick={onBack} disabled={connecting}>
          <ArrowLeft className="mr-2 h-4 w-4" />
          {t("setup.back")}
        </Button>
        <Button onClick={onConnect} disabled={!canConnect || connecting}>
          {connecting ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
          {t("setup.connectAndScan")}
        </Button>
      </div>
    </div>
  );
}
