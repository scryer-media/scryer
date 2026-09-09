import { AlertCircle, ExternalLink, Loader2, Merge } from "lucide-react";

import { ImportInstancePill } from "@/components/setup/import/import-instance-pill";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { canRetryProwlarrDiscovery } from "@/lib/external-import-wizard-orchestration";
import type { ImportInstanceKind } from "@/lib/hooks/use-external-import-setup";
import type { UseExternalImportSetupReturn } from "@/lib/hooks/use-external-import-setup";
import type {
  ExternalImportDownloadClient,
  ExternalImportIndexer,
} from "@/lib/types/external-import";
import { cn } from "@/lib/utils";
import { selectorId } from "@/lib/utils/dom-ids";

interface SetupImportSourcesViewProps {
  wizard: UseExternalImportSetupReturn;
  t: (key: string, values?: Record<string, unknown>) => string;
}

/** Download-client implementations that carry a key Scryer can't read back. */
const DC_TYPES_REQUIRING_API_KEY = new Set(["sabnzbd", "weaver"]);

/** Pill kind for a source key like "sonarr:http://host:8989" / "prowlarr:...". */
function pillKindForSourceKey(sourceKey: string): ImportInstanceKind | "manual" {
  const prefix = sourceKey.split(":", 1)[0]?.toLowerCase() ?? "";
  if (prefix === "sonarr") {
    return "SONARR";
  }
  if (prefix === "radarr") {
    return "RADARR";
  }
  if (prefix === "prowlarr") {
    return "PROWLARR";
  }
  return "manual";
}

/** Short, human label for a source-key pill (host/port if we can parse it). */
function sourceKeyLabel(sourceKey: string): string {
  const sep = sourceKey.indexOf(":");
  const rest = sep >= 0 ? sourceKey.slice(sep + 1) : sourceKey;
  const trimmed = rest.trim();
  if (!trimmed) return sourceKey;
  try {
    const url = new URL(trimmed);
    return url.port ? `${url.hostname}:${url.port}` : url.hostname;
  } catch {
    return trimmed;
  }
}

function downloadClientSubtitle(dc: ExternalImportDownloadClient): string {
  const where = dc.host
    ? `${dc.host}${dc.port ? `:${dc.port}` : ""}`
    : (dc.urlBase ?? "");
  return where ? `${dc.implementation} @ ${where}` : dc.implementation;
}

function indexerSubtitle(idx: ExternalImportIndexer): string {
  return idx.baseUrl ? `${idx.implementation} @ ${idx.baseUrl}` : idx.implementation;
}

function downloadClientNeedsApiKey(dc: ExternalImportDownloadClient): boolean {
  const clientType = dc.scryerClientType?.trim().toLowerCase() ?? null;
  return (
    dc.supported &&
    !dc.apiKeyPresent &&
    clientType !== null &&
    DC_TYPES_REQUIRING_API_KEY.has(clientType)
  );
}

/** Uppercase card-header label, matching the prototype's section headings. */
function SectionTitle({ children }: { children: React.ReactNode }) {
  return (
    <CardTitle className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
      {children}
    </CardTitle>
  );
}

/** "FROM <pill> <pill>" provenance line shown under merged/multi-source rows. */
function SourcePills({
  sourceKeys,
  t,
}: {
  sourceKeys: string[];
  t: SetupImportSourcesViewProps["t"];
}) {
  return (
    <div className="mt-2 flex flex-wrap items-center gap-1.5">
      <span className="text-[10px] font-bold uppercase tracking-[0.08em] text-[var(--scry-faint2)]">
        {t("setup.from")}
      </span>
      {sourceKeys.map((sourceKey) => (
        <ImportInstancePill
          key={sourceKey}
          kind={pillKindForSourceKey(sourceKey)}
          label={sourceKeyLabel(sourceKey)}
          title={sourceKey}
          size="sm"
        />
      ))}
    </div>
  );
}

export default function SetupImportSourcesView({
  wizard,
  t,
}: SetupImportSourcesViewProps) {
  const {
    preview,
    selectedDcKeys,
    selectedIdxKeys,
    dcApiKeyOverrides,
    dcPasswordOverrides,
    idxApiKeyOverrides,
    toggleDownloadClient,
    toggleIndexer,
    setDownloadClientApiKeyOverride,
    setDownloadClientPasswordOverride,
    setIndexerApiKeyOverride,
    prowlarrWarmupSessionId,
    prowlarrWarmupProgress,
    retryProwlarrWarmup,
  } = wizard;
  const prowlarrDiscoveryActive =
    Boolean(prowlarrWarmupSessionId) &&
    (!prowlarrWarmupProgress ||
      prowlarrWarmupProgress.status === "QUEUED" ||
      prowlarrWarmupProgress.status === "RUNNING");
  const prowlarrDiscoveryFailed = canRetryProwlarrDiscovery(
    prowlarrWarmupProgress?.status ?? null,
  );

  if (!preview) {
    return (
      <p
        id="setup-import-sources-loading"
        className="flex items-center justify-center gap-2 py-10 text-center text-sm text-muted-foreground"
      >
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("setup.sourcesLoading")}
      </p>
    );
  }

  return (
    <div id="setup-import-sources-view" className="flex flex-col gap-6">
      {prowlarrDiscoveryActive ? (
        <div
          data-slot="prowlarr-discovery-loading"
          className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm text-muted-foreground"
        >
          <Loader2 className="h-4 w-4 animate-spin" />
          Discovering Prowlarr indexers…
        </div>
      ) : null}
      {prowlarrDiscoveryFailed ? (
        <div
          data-slot="prowlarr-discovery-error"
          className="flex items-center justify-between gap-3 rounded-md border border-destructive/40 bg-destructive/5 px-3 py-2 text-sm"
        >
          <span className="flex items-center gap-2 text-destructive">
            <AlertCircle className="h-4 w-4" />
            {prowlarrWarmupProgress?.errorMessage ||
              "Prowlarr indexer discovery failed."}
          </span>
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => void retryProwlarrWarmup()}
          >
            Retry
          </Button>
        </div>
      ) : null}
      {/* ── Download Clients ─────────────────────────────────────────────── */}
      <Card data-slot="import-sources-clients">
        <CardHeader className="mb-3 flex items-center justify-between gap-3">
          <SectionTitle>{t("setup.downloadClients")}</SectionTitle>
          <span
            className="inline-flex items-center gap-1.5 rounded-md border px-2 py-0.5 text-[11px] font-medium"
            style={{
              background: "rgba(var(--scry-accent-rgb), 0.1)",
              borderColor: "var(--scry-baccent)",
              color: "var(--scry-accent-text)",
            }}
          >
            <Merge className="h-3 w-3" />
            {t("setup.duplicatesMerged")}
          </span>
        </CardHeader>
        <CardContent className="space-y-1">
          {preview.downloadClients.map((dc) => {
            const selected = selectedDcKeys.has(dc.dedupKey);
            const merged = dc.sourceKeys.length > 1;
            const needsApiKey = downloadClientNeedsApiKey(dc);
            const needsPassword = dc.supported && dc.requiresPasswordOverride;
            return (
              <div key={dc.dedupKey}>
                <label
                  id={selectorId("setup-import-source-client-row", dc.dedupKey)}
                  data-source-count={dc.sourceKeys.length}
                  data-source-keys={dc.sourceKeys.join("|")}
                  className={cn(
                    "flex items-start gap-3 rounded px-2 py-2 text-sm",
                    dc.supported
                      ? "cursor-pointer hover:bg-[var(--scry-hover)]"
                      : "cursor-not-allowed opacity-50",
                  )}
                >
                  <Checkbox
                    className="mt-0.5"
                    checked={selected}
                    onCheckedChange={() => toggleDownloadClient(dc.dedupKey)}
                    disabled={!dc.supported}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-semibold text-[var(--scry-ink2)]">
                        {dc.name}
                      </span>
                      {merged ? (
                        <Badge
                          tone="info"
                          className="gap-1 border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.1)] text-[10px] uppercase text-[var(--scry-accent-text)]"
                        >
                          <Merge className="h-2.5 w-2.5" />
                          {t("setup.merged")}
                        </Badge>
                      ) : null}
                      {!dc.supported ? (
                        <Badge tone="neutral" className="text-[10px] uppercase">
                          {t("setup.notSupported")}
                        </Badge>
                      ) : null}
                    </div>
                    <p className="truncate text-xs text-muted-foreground">
                      {downloadClientSubtitle(dc)}
                    </p>
                    {merged ? (
                      <SourcePills sourceKeys={dc.sourceKeys} t={t} />
                    ) : null}
                    {needsPassword && selected ? (
                      <div className="mt-2 space-y-1">
                        <p className="text-xs text-[var(--scry-warning-text)]">
                          {t("setup.passwordRequired")}
                        </p>
                        <Input
                          id={selectorId(
                            "setup-import-source-client-password",
                            dc.dedupKey,
                          )}
                          type="password"
                          ignorePasswordManagers
                          value={dcPasswordOverrides[dc.dedupKey] ?? ""}
                          onChange={(e) =>
                            setDownloadClientPasswordOverride(
                              dc.dedupKey,
                              e.target.value,
                            )
                          }
                          className="h-8 font-[var(--font-code)] text-xs"
                        />
                      </div>
                    ) : null}
                    {needsApiKey && selected ? (
                      <div className="mt-2 space-y-1">
                        <p className="text-xs text-muted-foreground">
                          {t("setup.apiKeyMasked")}
                        </p>
                        <Input
                          id={selectorId(
                            "setup-import-source-client-api-key",
                            dc.dedupKey,
                          )}
                          type="password"
                          ignorePasswordManagers
                          value={dcApiKeyOverrides[dc.dedupKey] ?? ""}
                          onChange={(e) =>
                            setDownloadClientApiKeyOverride(
                              dc.dedupKey,
                              e.target.value,
                            )
                          }
                          className="h-8 font-[var(--font-code)] text-xs"
                        />
                      </div>
                    ) : null}
                  </div>
                </label>
              </div>
            );
          })}
        </CardContent>
      </Card>

      {/* ── Indexers ─────────────────────────────────────────────────────── */}
      <Card data-slot="import-sources-indexers">
        <CardHeader className="mb-3">
          <SectionTitle>{t("setup.indexers")}</SectionTitle>
        </CardHeader>
        <CardContent className="space-y-1">
          {preview.indexers.map((idx) => {
            const selected = selectedIdxKeys.has(idx.dedupKey);
            const merged = idx.sourceKeys.length > 1;
            const needsApiKey = idx.supported && idx.requiresApiKeyOverride;
            return (
              <div key={idx.dedupKey}>
                <label
                  id={selectorId("setup-import-source-indexer-row", idx.dedupKey)}
                  className={cn(
                    "flex items-start gap-3 rounded px-2 py-2 text-sm",
                    idx.supported
                      ? "cursor-pointer hover:bg-[var(--scry-hover)]"
                      : "cursor-not-allowed opacity-50",
                  )}
                >
                  <Checkbox
                    className="mt-0.5"
                    checked={selected}
                    onCheckedChange={() => toggleIndexer(idx.dedupKey)}
                    disabled={!idx.supported}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-semibold text-[var(--scry-ink2)]">
                        {idx.name}
                      </span>
                      {merged ? (
                        <Badge
                          tone="info"
                          className="gap-1 border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.1)] text-[10px] uppercase text-[var(--scry-accent-text)]"
                        >
                          <Merge className="h-2.5 w-2.5" />
                          {t("setup.merged")}
                        </Badge>
                      ) : null}
                      {!idx.supported ? (
                        <Badge tone="neutral" className="text-[10px] uppercase">
                          {t("setup.notSupported")}
                        </Badge>
                      ) : null}
                    </div>
                    <p className="truncate text-xs text-muted-foreground">
                      {indexerSubtitle(idx)}
                    </p>
                    {merged ? (
                      <SourcePills sourceKeys={idx.sourceKeys} t={t} />
                    ) : null}
                    {needsApiKey && selected ? (
                      <div className="mt-2 space-y-1">
                        <p className="text-xs text-muted-foreground">
                          {t("setup.apiKeyMasked")}
                        </p>
                        <Input
                          id={selectorId(
                            "setup-import-source-indexer-api-key",
                            idx.dedupKey,
                          )}
                          type="password"
                          ignorePasswordManagers
                          value={idxApiKeyOverrides[idx.dedupKey] ?? ""}
                          onChange={(e) =>
                            setIndexerApiKeyOverride(idx.dedupKey, e.target.value)
                          }
                          className="h-8 font-[var(--font-code)] text-xs"
                        />
                        {idx.apiKeyHelpUrl ? (
                          <a
                            href={idx.apiKeyHelpUrl}
                            target="_blank"
                            rel="noopener noreferrer"
                            className="inline-flex items-center gap-1 text-xs text-primary underline underline-offset-2"
                          >
                            {t("setup.indexerApiKeyHelpLink")}
                            <ExternalLink className="h-3 w-3" />
                          </a>
                        ) : null}
                      </div>
                    ) : null}
                  </div>
                </label>
              </div>
            );
          })}
        </CardContent>
      </Card>
    </div>
  );
}
