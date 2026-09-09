import * as React from "react";
import { DatabaseBackup, Loader2, LockKeyhole, RotateCcw, Upload } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  SetupBackButton,
  SetupPanel,
  SetupPrimaryButton,
  SetupStepHeader,
} from "./setup-chrome";
import { Card, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { scryerFetch } from "@/lib/graphql/urql-client";
import { getAuthToken } from "@/lib/hooks/use-auth";
import { getRuntimeBasePath, getRuntimeGraphqlUrl } from "@/lib/runtime-config";
import { cn } from "@/lib/utils";

const INSPECT_RESTORE_BUNDLE_OPERATION = `
  mutation InspectRestoreBundle($bundleUpload: Upload!, $password: String) {
    inspectRestoreBundle(bundleUpload: $bundleUpload, password: $password) {
      uploadId
      summary {
        formatVersion
        createdAt
        sourceScryerVersion
        sourceEngine
        sourceMigrationKey
        encrypted
        rowCounts {
          table
          rowCount
        }
        totalRows
      }
    }
  }
`;

const APPLY_RESTORE_BUNDLE_OPERATION = `
  mutation ApplyRestoreBundle($uploadId: ID!, $password: String) {
    applyRestoreBundle(uploadId: $uploadId, password: $password) {
      summary {
        formatVersion
      }
    }
  }
`;

type RestoreSummaryPayload = {
  formatVersion: string;
  createdAt: string;
  sourceScryerVersion: string;
  sourceEngine: string;
  sourceMigrationKey: string | null;
  encrypted: boolean;
  rowCounts: Array<{
    table: string;
    rowCount: number;
  }>;
  totalRows: number;
};

type RestoreInspectResponse = {
  data?: {
    inspectRestoreBundle?: {
      uploadId: string;
      summary: RestoreSummaryPayload;
    } | null;
  };
  errors?: Array<{
    message: string;
  }>;
};

type RestoreApplyResponse = {
  data?: {
    applyRestoreBundle?: {
      summary: {
        formatVersion: string;
      };
    } | null;
  };
  errors?: Array<{
    message: string;
  }>;
};

interface SetupRestoreViewProps {
  t: (
    key: string,
    values?: Record<string, string | number | boolean | null | undefined>,
  ) => string;
  onBack: () => void;
  onBackendRestarting: () => void;
}

function buildFrontendUrl(path: string): string {
  const basePath = getRuntimeBasePath();
  if (basePath === "/") {
    return path;
  }
  return path === "/" ? basePath : `${basePath}${path}`;
}

function graphqlUrl(): string {
  return new URL(getRuntimeGraphqlUrl(), window.location.origin).toString();
}

function createAuthHeaders(init?: HeadersInit): Headers {
  const headers = new Headers(init);
  const token = getAuthToken();
  if (token) {
    headers.set("authorization", `Bearer ${token}`);
  }
  return headers;
}

function graphqlErrorMessage(
  payload: { errors?: Array<{ message: string }> } | undefined,
): string | null {
  return payload?.errors?.find((entry) => entry.message.trim())?.message.trim() ?? null;
}

async function parseJsonResponse<T>(response: Response): Promise<T | undefined> {
  const text = await response.text().catch(() => "");
  if (!text.trim()) {
    return undefined;
  }

  return JSON.parse(text) as T;
}

function formatDateTime(value: string): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return value;
  }
  return new Date(timestamp).toLocaleString();
}

export function SetupRestoreView({
  t,
  onBack,
  onBackendRestarting,
}: SetupRestoreViewProps) {
  const [selectedBundle, setSelectedBundle] = React.useState<File | null>(null);
  const [password, setPassword] = React.useState("");
  const [fileInputKey, setFileInputKey] = React.useState(0);
  const [inspecting, setInspecting] = React.useState(false);
  const [applying, setApplying] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [uploadId, setUploadId] = React.useState<string | null>(null);
  const [summary, setSummary] = React.useState<RestoreSummaryPayload | null>(null);
  const [isDragActive, setIsDragActive] = React.useState(false);
  const fileInputRef = React.useRef<HTMLInputElement | null>(null);

  const bundleLooksEncrypted
    = selectedBundle?.name.toLowerCase().endsWith(".enc") ?? false;
  const requiresPassword = summary?.encrypted ?? bundleLooksEncrypted;
  const rowCounts = summary
    ? [...summary.rowCounts].sort((left, right) => left.table.localeCompare(right.table))
    : [];

  const handleBundleSelected = React.useCallback((file: File | null) => {
    setError(null);
    setSelectedBundle(file);
    setUploadId(null);
    setSummary(null);
    if (!file?.name.toLowerCase().endsWith(".enc")) {
      setPassword("");
    }
  }, []);

  const resetSelection = React.useCallback(() => {
    setSelectedBundle(null);
    setPassword("");
    setUploadId(null);
    setSummary(null);
    setError(null);
    setIsDragActive(false);
    setFileInputKey((current) => current + 1);
  }, []);

  const handleInspect = React.useCallback(async () => {
    if (!selectedBundle) {
      setError(t("setup.restoreNoFile"));
      return;
    }
    if (requiresPassword && password.length === 0) {
      setError(t("setup.restorePasswordRequired"));
      return;
    }

    setInspecting(true);
    setError(null);
    try {
      const formData = new FormData();
      formData.append(
        "operations",
        JSON.stringify({
          query: INSPECT_RESTORE_BUNDLE_OPERATION,
          variables: {
            bundleUpload: null,
            password: password.length > 0 ? password : null,
          },
        }),
      );
      formData.append("map", JSON.stringify({ "0": ["variables.bundleUpload"] }));
      formData.append("0", selectedBundle, selectedBundle.name);

      const response = await scryerFetch(graphqlUrl(), {
        method: "POST",
        headers: createAuthHeaders(),
        body: formData,
      });
      const payload = await parseJsonResponse<RestoreInspectResponse>(response);
      if (!response.ok) {
        throw new Error(
          graphqlErrorMessage(payload) ?? t("status.failedToLoad"),
        );
      }
      const result = payload?.data?.inspectRestoreBundle;
      if (!result) {
        throw new Error(graphqlErrorMessage(payload) ?? t("status.failedToLoad"));
      }

      setUploadId(result.uploadId);
      setSummary(result.summary);
    } catch (nextError) {
      setError(
        nextError instanceof Error && nextError.message.trim()
          ? nextError.message.trim()
          : t("status.failedToLoad"),
      );
    } finally {
      setInspecting(false);
    }
  }, [password, requiresPassword, selectedBundle, t]);

  const handleApply = React.useCallback(async () => {
    if (!uploadId || !summary) {
      return;
    }

    setApplying(true);
    setError(null);
    try {
      const response = await scryerFetch(graphqlUrl(), {
        method: "POST",
        headers: createAuthHeaders({
          "content-type": "application/json",
        }),
        body: JSON.stringify({
          query: APPLY_RESTORE_BUNDLE_OPERATION,
          variables: {
            uploadId,
            password: summary.encrypted ? password : null,
          },
        }),
      });
      const payload = await parseJsonResponse<RestoreApplyResponse>(response);
      if (!response.ok) {
        throw new Error(
          graphqlErrorMessage(payload) ?? t("status.failedToUpdate"),
        );
      }
      if (!payload?.data?.applyRestoreBundle) {
        throw new Error(graphqlErrorMessage(payload) ?? t("status.failedToUpdate"));
      }

      window.history.replaceState(null, "", buildFrontendUrl("/"));
      onBackendRestarting();
    } catch (nextError) {
      setApplying(false);
      setError(
        nextError instanceof Error && nextError.message.trim()
          ? nextError.message.trim()
          : t("status.failedToUpdate"),
      );
    }
  }, [onBackendRestarting, password, summary, t, uploadId]);

  return (
    <SetupPanel className="w-full space-y-6 bg-[linear-gradient(180deg,rgba(12,18,35,0.82),rgba(8,13,26,0.72))]">
      <SetupStepHeader
        icon={DatabaseBackup}
        title={t("setup.restoreTitle")}
        subtitle={t("setup.restoreDescription")}
      />

      {!summary ? (
        <Card className="border-[var(--scry-border2)] bg-[rgba(10,17,32,0.55)] p-0 shadow-none">
          <CardContent className="space-y-5 p-5 sm:p-6">
            <div className="space-y-3 text-sm">
              <span className="text-sm font-semibold text-[var(--scry-ink2)]">
                {t("setup.restoreSelectBundle")}
              </span>
              <input
                id="setup-restore-file-input"
                key={fileInputKey}
                ref={fileInputRef}
                type="file"
                accept=".tar.zst,.enc,.zst"
                className="hidden"
                onChange={(event) => {
                  handleBundleSelected(event.target.files?.[0] ?? null);
                }}
                disabled={inspecting}
              />
              <div
                onDragOver={(event) => {
                  event.preventDefault();
                  event.dataTransfer.dropEffect = "copy";
                  setIsDragActive(true);
                }}
                onDragLeave={(event) => {
                  event.preventDefault();
                  if (event.currentTarget.contains(event.relatedTarget as Node | null)) {
                    return;
                  }
                  setIsDragActive(false);
                }}
                onDrop={(event) => {
                  event.preventDefault();
                  setIsDragActive(false);
                  handleBundleSelected(event.dataTransfer.files?.[0] ?? null);
                }}
                className={cn(
                  "rounded-[16px] border border-dashed px-5 py-8 transition-colors",
                  isDragActive
                    ? "border-[var(--scry-accent-text)] bg-[rgba(var(--scry-accent-rgb),0.12)]"
                    : "border-[rgba(var(--scry-accent-rgb),0.34)] bg-[rgba(var(--scry-accent-rgb),0.045)] hover:border-[var(--scry-baccent)] hover:bg-[rgba(var(--scry-accent-rgb),0.075)]",
                )}
              >
                <div className="mx-auto flex max-w-xl flex-col items-center gap-4 text-center">
                  <div className="flex h-12 w-12 items-center justify-center rounded-[14px] border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.12)] text-[var(--scry-accent-text)]">
                    <Upload className="h-5 w-5" />
                  </div>
                  <div className="space-y-1">
                    <p
                      className={cn(
                        "break-all text-base font-semibold text-[var(--scry-ink2)]",
                        selectedBundle ? "font-[var(--font-code)] text-sm" : null,
                      )}
                    >
                      {selectedBundle
                        ? selectedBundle.name
                        : t("setup.restoreDropTargetTitle")}
                    </p>
                    <p className="text-sm leading-relaxed text-[var(--scry-muted)]">
                      {selectedBundle
                        ? bundleLooksEncrypted
                          ? t("setup.restoreDropTargetEncryptedSelected")
                          : t("setup.restoreDropTargetSelected")
                        : t("setup.restoreDropTargetDescription")}
                    </p>
                  </div>
                  <div className="flex flex-wrap items-center justify-center gap-3">
                    <SetupPrimaryButton
                      id="setup-restore-select-file"
                      type="button"
                      onClick={() => fileInputRef.current?.click()}
                      disabled={inspecting}
                    >
                      {t("setup.restoreSelectFile")}
                    </SetupPrimaryButton>
                    {selectedBundle ? (
                      <Button
                        id="setup-restore-clear-file"
                        type="button"
                        variant="outline"
                        onClick={resetSelection}
                        disabled={inspecting}
                        className="border-[var(--scry-danger-border)] text-[var(--scry-danger-text)] hover:bg-[var(--scry-danger-bg)] hover:text-[var(--scry-danger-text)]"
                      >
                        {t("setup.restoreClearFile")}
                      </Button>
                    ) : null}
                  </div>
                  <p className="text-xs text-[var(--scry-faint)]">
                    {t("setup.restoreDropTargetFormats")}
                  </p>
                </div>
              </div>
            </div>

            {requiresPassword ? (
              <label className="block space-y-2 text-sm">
                <span className="font-semibold text-[var(--scry-ink2)]">
                  {t("settings.password")}
                </span>
                <Input
                  id="setup-restore-password"
                  type="password"
                  ignorePasswordManagers
                  value={password}
                  onChange={(event) => {
                    setError(null);
                    setPassword(event.target.value);
                  }}
                  placeholder={t("settings.password")}
                  disabled={inspecting}
                  className="border-[var(--scry-border2)] bg-[var(--scry-page2)]"
                />
              </label>
            ) : null}

            {requiresPassword ? (
              <div className="rounded-[12px] border border-[var(--scry-border2)] bg-[rgba(10,17,32,0.48)] p-3 text-xs text-[var(--scry-muted)]">
                <div className="flex items-center gap-2 text-[var(--scry-ink2)]">
                  <LockKeyhole className="h-3.5 w-3.5" />
                  <span>{t("setup.restorePasswordHelp")}</span>
                </div>
              </div>
            ) : null}

            {error ? (
              <p id="setup-restore-error" className="text-sm text-destructive">{error}</p>
            ) : null}
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-4">
          <Card className="border-[var(--scry-border2)] bg-[rgba(10,17,32,0.55)] p-0 shadow-none">
            <CardContent className="space-y-5 p-5 sm:p-6">
              <div className="flex flex-wrap items-center gap-2">
                <span className="rounded-full border border-[var(--scry-baccent)] bg-[rgba(var(--scry-accent-rgb),0.11)] px-2.5 py-1 text-xs font-semibold text-[var(--scry-accent-text)]">
                  {summary.encrypted
                    ? t("settings.backupsEncrypted")
                    : t("settings.backupsPlaintext")}
                </span>
                <span className="rounded-full border border-[var(--scry-border2)] bg-[var(--scry-page2)] px-2.5 py-1 font-[var(--font-code)] text-xs font-medium text-[var(--scry-muted2)]">
                  {summary.formatVersion}
                </span>
              </div>

              <div className="grid gap-3 text-sm sm:grid-cols-2">
                <div className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-page2)] p-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--scry-faint)]">
                    {t("setup.restoreCreatedAt")}
                  </p>
                  <p className="mt-1 text-[var(--scry-ink2)]">
                    {formatDateTime(summary.createdAt)}
                  </p>
                </div>
                <div className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-page2)] p-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--scry-faint)]">
                    {t("setup.restoreSourceVersion")}
                  </p>
                  <p className="mt-1 font-[var(--font-code)] text-sm text-[var(--scry-ink2)]">
                    {summary.sourceScryerVersion}
                  </p>
                </div>
                <div className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-page2)] p-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--scry-faint)]">
                    {t("setup.restoreSourceEngine")}
                  </p>
                  <p className="mt-1 font-[var(--font-code)] text-sm text-[var(--scry-ink2)]">
                    {summary.sourceEngine}
                  </p>
                </div>
                <div className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-page2)] p-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--scry-faint)]">
                    {t("setup.restoreMigrationKey")}
                  </p>
                  <p className="mt-1 font-[var(--font-code)] text-sm text-[var(--scry-ink2)]">
                    {summary.sourceMigrationKey ?? "-"}
                  </p>
                </div>
                <div className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-page2)] p-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--scry-faint)]">
                    {t("setup.restoreTotalRows")}
                  </p>
                  <p className="mt-1 font-[var(--font-space-grotesk)] text-lg font-bold text-[var(--scry-ink2)]">
                    {Number(summary.totalRows).toLocaleString()}
                  </p>
                </div>
                <div className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-page2)] p-3">
                  <p className="text-xs uppercase tracking-wide text-[var(--scry-faint)]">
                    Tables
                  </p>
                  <p className="mt-1 font-[var(--font-space-grotesk)] text-lg font-bold text-[var(--scry-ink2)]">
                    {rowCounts.length.toLocaleString()}
                  </p>
                </div>
              </div>

              <div className="space-y-2">
                <h3 className="text-sm font-semibold text-[var(--scry-ink2)]">
                  {t("setup.restoreSummaryTitle")}
                </h3>
                <div className="max-h-64 overflow-y-auto rounded-[12px] border border-[var(--scry-border2)] bg-[var(--scry-page2)]">
                  <Table>
                    <TableHeader>
                      <TableRow>
                        <TableHead className="text-[var(--scry-muted2)]">
                          Table
                        </TableHead>
                        <TableHead className="text-right text-[var(--scry-muted2)]">
                          Rows
                        </TableHead>
                      </TableRow>
                    </TableHeader>
                    <TableBody>
                      {rowCounts.map((entry) => (
                        <TableRow key={entry.table}>
                          <TableCell className="font-[var(--font-code)] text-xs text-[var(--scry-ink2)]">
                            {entry.table}
                          </TableCell>
                          <TableCell className="text-right text-xs text-[var(--scry-muted)]">
                            {Number(entry.rowCount).toLocaleString()}
                          </TableCell>
                        </TableRow>
                      ))}
                    </TableBody>
                  </Table>
                </div>
              </div>

              <p className="rounded-[12px] border border-[var(--scry-border)] bg-[var(--scry-page2)] p-3 text-sm leading-relaxed text-[var(--scry-muted)]">
                {t("setup.restoreConfirmDescription")}
              </p>

              {error ? (
                <p id="setup-restore-error" className="text-sm text-destructive">{error}</p>
              ) : null}
            </CardContent>
          </Card>
        </div>
      )}

      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-[var(--scry-border)] pt-2">
        <SetupBackButton
          id="setup-restore-back"
          onClick={onBack}
          disabled={inspecting || applying}
        >
          {t("setup.back")}
        </SetupBackButton>

        <div className="flex flex-wrap items-center gap-2">
          {summary ? (
            <Button
              id="setup-restore-choose-another"
              type="button"
              variant="outline"
              onClick={resetSelection}
              disabled={applying}
              className="border-[var(--scry-border2)] bg-[var(--scry-page2)]"
            >
              <RotateCcw className="h-4 w-4" />
              {t("setup.restoreChooseAnother")}
            </Button>
          ) : null}

          {summary ? (
            <SetupPrimaryButton
              id="setup-restore-apply"
              type="button"
              onClick={() => void handleApply()}
              disabled={applying}
            >
              {applying ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Upload className="h-4 w-4" />
              )}
              {t("setup.restoreApply")}
            </SetupPrimaryButton>
          ) : (
            <SetupPrimaryButton
              id="setup-restore-inspect"
              type="button"
              onClick={() => void handleInspect()}
              disabled={
                inspecting
                || !selectedBundle
                || (requiresPassword && password.length === 0)
              }
            >
              {inspecting ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Upload className="h-4 w-4" />
              )}
              {t("setup.restoreInspect")}
            </SetupPrimaryButton>
          )}
        </div>
      </div>
    </SetupPanel>
  );
}
