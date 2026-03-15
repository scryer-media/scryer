import { ArrowLeft, Check, Loader2, Ban } from "lucide-react";
import { Button } from "@/components/ui/button";
import type { ExternalImportPreview } from "@/lib/types/external-import";

interface SetupImportReviewViewProps {
  t: (key: string) => string;
  preview: ExternalImportPreview;
  selectedMoviesPath: string | null;
  selectedSeriesPath: string | null;
  selectedAnimePath: string | null;
  selectedDcKeys: Set<string>;
  selectedIdxKeys: Set<string>;
  dcApiKeyOverrides: Map<string, string>;
  onSelectMoviesPath: (path: string | null) => void;
  onSelectSeriesPath: (path: string | null) => void;
  onSelectAnimePath: (path: string | null) => void;
  onToggleDc: (dedupKey: string) => void;
  onToggleIdx: (dedupKey: string) => void;
  onSetDcApiKey: (dedupKey: string, apiKey: string) => void;
  onImport: () => void;
  onBack: () => void;
  importing: boolean;
  error: string | null;
}

export function SetupImportReviewView({
  t,
  preview,
  selectedMoviesPath,
  selectedSeriesPath,
  selectedAnimePath,
  selectedDcKeys,
  selectedIdxKeys,
  dcApiKeyOverrides,
  onSelectMoviesPath,
  onSelectSeriesPath,
  onSelectAnimePath,
  onToggleDc,
  onToggleIdx,
  onSetDcApiKey,
  onImport,
  onBack,
  importing,
  error,
}: SetupImportReviewViewProps) {
  const sonarrFolders = preview.rootFolders.filter((f) => f.source === "sonarr");
  const radarrFolders = preview.rootFolders.filter((f) => f.source === "radarr");

  const hasAnySelection =
    selectedMoviesPath !== null ||
    selectedSeriesPath !== null ||
    selectedAnimePath !== null ||
    selectedDcKeys.size > 0 ||
    selectedIdxKeys.size > 0;

  return (
    <div className="w-full space-y-6">
      <div className="text-center">
        <h2 className="mb-2 text-xl font-semibold">{t("setup.reviewTitle")}</h2>
        <p className="text-sm text-muted-foreground">{t("setup.reviewDescription")}</p>
      </div>

      {/* Connection badges */}
      <div className="flex items-center justify-center gap-3">
        {preview.sonarrConnected ? (
          <span className="inline-flex items-center gap-1.5 rounded-full bg-blue-500/10 px-3 py-1 text-xs font-medium text-blue-600 dark:text-blue-400">
            <Check className="h-3 w-3" />
            Sonarr {preview.sonarrVersion ? `v${preview.sonarrVersion}` : ""} {t("setup.connected")}
          </span>
        ) : null}
        {preview.radarrConnected ? (
          <span className="inline-flex items-center gap-1.5 rounded-full bg-amber-500/10 px-3 py-1 text-xs font-medium text-amber-600 dark:text-amber-400">
            <Check className="h-3 w-3" />
            Radarr {preview.radarrVersion ? `v${preview.radarrVersion}` : ""} {t("setup.connected")}
          </span>
        ) : null}
      </div>

      {/* Media Paths */}
      {(sonarrFolders.length > 0 || radarrFolders.length > 0) ? (
        <Section title={t("setup.mediaPathsSection")}>
          {radarrFolders.length > 0 ? (
            <div className="space-y-1">
              <p className="text-xs font-medium text-muted-foreground">
                {t("setup.moviesPathFrom")}
              </p>
              {radarrFolders.map((folder) => (
                <label
                  key={folder.path}
                  className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-muted"
                >
                  <input
                    type="radio"
                    name="movies-path"
                    checked={selectedMoviesPath === folder.path}
                    onChange={() => onSelectMoviesPath(folder.path)}
                    className="accent-primary"
                  />
                  <code className="text-xs">{folder.path}</code>
                </label>
              ))}
            </div>
          ) : null}
          {sonarrFolders.length > 0 ? (
            <div className="space-y-1">
              <p className="text-xs font-medium text-muted-foreground">
                {t("setup.seriesPathFrom")}
              </p>
              {sonarrFolders.map((folder) => (
                <label
                  key={folder.path}
                  className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-muted"
                >
                  <input
                    type="radio"
                    name="series-path"
                    checked={selectedSeriesPath === folder.path}
                    onChange={() => onSelectSeriesPath(folder.path)}
                    className="accent-primary"
                  />
                  <code className="text-xs">{folder.path}</code>
                </label>
              ))}
            </div>
          ) : null}
          {sonarrFolders.length > 1 ? (
            <div className="space-y-1">
              <p className="text-xs font-medium text-muted-foreground">
                {t("setup.animePathFrom")}
              </p>
              <label className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-muted">
                <input
                  type="radio"
                  name="anime-path"
                  checked={selectedAnimePath === null}
                  onChange={() => onSelectAnimePath(null)}
                  className="accent-primary"
                />
                <span className="text-xs text-muted-foreground">{t("setup.none")}</span>
              </label>
              {sonarrFolders.map((folder) => (
                <label
                  key={folder.path}
                  className="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-muted"
                >
                  <input
                    type="radio"
                    name="anime-path"
                    checked={selectedAnimePath === folder.path}
                    onChange={() => onSelectAnimePath(folder.path)}
                    className="accent-primary"
                  />
                  <code className="text-xs">{folder.path}</code>
                </label>
              ))}
            </div>
          ) : null}
        </Section>
      ) : null}

      {/* Download Clients */}
      {preview.downloadClients.length > 0 ? (
        <Section title={t("setup.downloadClientsSection")}>
          {preview.downloadClients.map((dc) => {
            const needsApiKey =
              dc.supported &&
              dc.apiKey === null &&
              (dc.scryerClientType === "sabnzbd" || dc.scryerClientType === "weaver");
            const isSelected = selectedDcKeys.has(dc.dedupKey);
            const sabUrl = dc.host
              ? `${dc.useSsl ? "https" : "http"}://${dc.host}${dc.port ? `:${dc.port}` : ""}${dc.urlBase ? `/${dc.urlBase.replace(/^\//, "")}` : ""}/config/general/`
              : null;
            return (
              <div key={dc.dedupKey}>
                <label
                  className={`flex items-center gap-3 rounded px-2 py-2 text-sm ${
                    dc.supported ? "cursor-pointer hover:bg-muted" : "cursor-not-allowed opacity-50"
                  }`}
                >
                  <input
                    type="checkbox"
                    checked={isSelected}
                    onChange={() => onToggleDc(dc.dedupKey)}
                    disabled={!dc.supported}
                    className="accent-primary"
                  />
                  <div className="flex-1">
                    <span className="font-medium">{dc.name}</span>
                    <span className="ml-2 text-xs text-muted-foreground">
                      {dc.implementation}
                      {dc.host ? ` @ ${dc.host}${dc.port ? `:${dc.port}` : ""}` : ""}
                    </span>
                  </div>
                  <SourceBadges sources={dc.sources} t={t} />
                  {!dc.supported ? (
                    <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                      {t("setup.notSupported")}
                    </span>
                  ) : null}
                </label>
                {needsApiKey && isSelected ? (
                  <div className="ml-8 mb-1 space-y-1">
                    <p className="text-xs text-muted-foreground">{t("setup.apiKeyMasked")}</p>
                    <input
                      type="text"
                      value={dcApiKeyOverrides.get(dc.dedupKey) ?? ""}
                      onChange={(e) => onSetDcApiKey(dc.dedupKey, e.target.value)}
                      placeholder={t("setup.apiKeyPlaceholder")}
                      className="w-full rounded border border-border bg-background px-2 py-1 font-mono text-xs outline-none focus:ring-1 focus:ring-primary"
                    />
                    {sabUrl ? (
                      <a
                        href={sabUrl}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="inline-block text-xs text-primary underline underline-offset-2"
                      >
                        {t("setup.apiKeyHelpLink")}
                      </a>
                    ) : null}
                  </div>
                ) : null}
              </div>
            );
          })}
        </Section>
      ) : null}

      {/* Indexers */}
      {preview.indexers.length > 0 ? (
        <Section title={t("setup.indexersSection")}>
          {preview.indexers.map((idx) => (
            <label
              key={idx.dedupKey}
              className={`flex items-center gap-3 rounded px-2 py-2 text-sm ${
                idx.supported ? "cursor-pointer hover:bg-muted" : "cursor-not-allowed opacity-50"
              }`}
            >
              <input
                type="checkbox"
                checked={selectedIdxKeys.has(idx.dedupKey)}
                onChange={() => onToggleIdx(idx.dedupKey)}
                disabled={!idx.supported}
                className="accent-primary"
              />
              <div className="flex-1">
                <span className="font-medium">{idx.name}</span>
                <span className="ml-2 text-xs text-muted-foreground">
                  {idx.implementation}
                </span>
              </div>
              <SourceBadges sources={idx.sources} t={t} />
              {!idx.supported ? (
                <span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
                  {t("setup.notSupported")}
                </span>
              ) : null}
            </label>
          ))}
        </Section>
      ) : null}

      {/* Nothing found */}
      {preview.downloadClients.length === 0 &&
        preview.indexers.length === 0 &&
        preview.rootFolders.length === 0 ? (
        <p className="py-4 text-center text-sm text-muted-foreground">
          <Ban className="mb-1 inline-block h-4 w-4" /> {t("setup.noItemsFound")}
        </p>
      ) : null}

      <p className="text-center text-xs text-muted-foreground">
        {t("setup.customFormatsHint")}
      </p>

      {error ? (
        <p className="text-center text-sm text-destructive">{error}</p>
      ) : null}

      <div className="flex items-center justify-between">
        <Button variant="ghost" onClick={onBack} disabled={importing}>
          <ArrowLeft className="mr-2 h-4 w-4" />
          {t("setup.back")}
        </Button>
        <Button onClick={onImport} disabled={!hasAnySelection || importing}>
          {importing ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : null}
          {importing ? t("setup.importing") : t("setup.importSelected")}
        </Button>
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2 rounded-lg border border-border p-4">
      <h3 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {title}
      </h3>
      {children}
    </div>
  );
}

function SourceBadges({ sources, t }: { sources: string[]; t: (key: string) => string }) {
  return (
    <span className="flex gap-1">
      {sources.map((source) => {
        const isSonarr = source === "sonarr";
        return (
          <span
            key={source}
            className={`rounded px-1.5 py-0.5 text-[10px] font-medium ${
              isSonarr
                ? "bg-blue-500/10 text-blue-600 dark:text-blue-400"
                : "bg-amber-500/10 text-amber-600 dark:text-amber-400"
            }`}
          >
            {isSonarr ? t("setup.fromSonarr") : t("setup.fromRadarr")}
          </span>
        );
      })}
    </span>
  );
}
