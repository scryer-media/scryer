import * as React from "react";
import {
  Download,
  Eye,
  EyeOff,
  FolderOpen,
  Loader2,
  LockKeyhole,
  Plus,
  RotateCcw,
  Trash2,
} from "lucide-react";
import { useBeforeUnload, useBlocker } from "react-router";
import { useClient } from "urql";
import { ConfirmDialog } from "@/components/common/confirm-dialog";
import { InfoHelp } from "@/components/common/info-help";
import { FolderBrowserDialog } from "@/components/setup/folder-browser-dialog";
import { SettingsToggleSwitch } from "@/components/common/settings-toggle-switch";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Table,
  TableBody,
  TableCell,
  TableCheckboxCell,
  TableCheckboxHead,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { TimePicker } from "@/components/ui/time-picker";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { selectorId } from "@/lib/utils/dom-ids";
import {
  createBackupMutation,
  deleteBackupMutation,
  prepareBackupDownloadMutation,
  updateAutoBackupSettingsMutation,
  updateBackupSettingsMutation,
} from "@/lib/graphql/mutations";
import { autoBackupSettingsQuery, backupSettingsQuery, backupsQuery } from "@/lib/graphql/queries";
import { scryerFetch } from "@/lib/graphql/urql-client";
import { useSettingsSubscription } from "@/lib/hooks/use-settings-subscription";
import { getRuntimeBasePath } from "@/lib/runtime-config";
import { formatUiDateTime } from "@/lib/utils/date-format";
import type {
  AutoBackupSettings,
  BackupSettings,
  UiDateTimeFormat,
} from "@/lib/types/settings";

type BackupRowCount = {
  table: string;
  rowCount: string;
};

type BackupTrigger = "manual" | "auto";

type BackupInfoRecord = {
  filename: string;
  sizeBytes: number;
  createdAt: string;
  formatVersion: string;
  sourceEngine: string;
  sourceMigrationKey: string | null;
  trigger: BackupTrigger;
  encrypted: boolean;
  rowCounts: BackupRowCount[];
  status: "creating" | "ready" | "invalid" | "failed";
  errorMessage: string | null;
};

type BackupsQueryResult = {
  backups?: BackupInfoRecord[];
};

type CreateBackupMutationResult = {
  createBackup?: BackupInfoRecord;
};

type DeleteBackupMutationResult = {
  deleteBackup?: {
    filename: string;
    deleted: boolean;
  };
};

type PrepareBackupDownloadMutationResult = {
  prepareBackupDownload?: {
    downloadUrl: string;
    downloadAuthorizationToken: string;
    expiresAt: string;
  };
};

type AutoBackupSettingsQueryResult = {
  autoBackupSettings?: AutoBackupSettings;
};

type BackupSettingsQueryResult = {
  backupSettings?: BackupSettings;
};

type UpdateAutoBackupSettingsMutationResult = {
  updateAutoBackupSettings?: AutoBackupSettings;
};

type UpdateBackupSettingsMutationResult = {
  updateBackupSettings?: BackupSettings;
};

const DEFAULT_AUTO_BACKUP_SETTINGS: AutoBackupSettings = {
  enabled: false,
  dailyTimeLocal: "03:00",
  autoBackupKeyPresent: false,
  autoBackupDisabledMissingKeyNotice: false,
  nextRunAt: null,
};
const DEFAULT_BACKUP_SETTINGS: BackupSettings = {
  customBackupPath: null,
  defaultBackupPath: "",
  effectiveBackupPath: "",
};
const AUTO_BACKUP_KEY_MIN_LENGTH = 8;
const BACKUPS_PANEL_CLASS =
  "overflow-hidden rounded-[14px] border border-[var(--scry-border)] bg-[var(--scry-surf)] shadow-[0_10px_24px_rgba(0,0,0,0.16)]";
const BACKUPS_PANEL_HEADER_CLASS =
  "border-b border-[var(--scry-border3)] bg-[linear-gradient(180deg,rgba(255,255,255,0.035),rgba(255,255,255,0))] px-4 py-3";
const BACKUPS_PANEL_TITLE_CLASS =
  "text-[15px] font-semibold text-[var(--scry-ink2)]";
const BACKUPS_PANEL_BODY_CLASS = "p-4 sm:p-5";
const BACKUPS_INSET_CLASS =
  "rounded-[12px] border border-[var(--scry-line2)] bg-[var(--scry-card2)]";
const BACKUPS_MUTED_TEXT_CLASS = "text-[var(--scry-muted3)]";

type SaveFilePickerWindow = Window & {
  showSaveFilePicker?: (options: {
    suggestedName: string;
  }) => Promise<{
    createWritable: () => Promise<WritableStream<Uint8Array>>;
  }>;
};

function mutationErrorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error && error.message.trim()) {
    return error.message.trim();
  }
  return fallback;
}

function nonWhitespaceCharacterCount(value: string): number {
  return Array.from(value.trim()).filter((char) => !/\s/u.test(char)).length;
}

function buildAppUrl(path: string): string {
  const basePath = getRuntimeBasePath();
  return basePath === "/" ? path : `${basePath}${path}`;
}

async function readResponseErrorMessage(response: Response, fallback: string): Promise<string> {
  const contentType = response.headers.get("content-type") ?? "";
  if (contentType.includes("application/json")) {
    const payload = await response.json().catch(() => null) as {
      error?: string;
      error_id?: string;
    } | null;
    const message = payload?.error?.trim();
    if (message) {
      const errorId = payload?.error_id?.trim();
      return errorId ? `${message}. Reference ID: ${errorId}` : message;
    }
  }

  const text = await response.text().catch(() => "");
  return text.trim() || fallback;
}

async function saveDownloadResponse(response: Response, filename: string): Promise<void> {
  const windowWithPicker = window as SaveFilePickerWindow;
  if (response.body && typeof windowWithPicker.showSaveFilePicker === "function") {
    try {
      const handle = await windowWithPicker.showSaveFilePicker({
        suggestedName: filename,
      });
      const writable = await handle.createWritable();
      await response.body.pipeTo(writable);
      return;
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") {
        return;
      }
      throw error;
    }
  }

  const blob = await response.blob();
  const downloadUrl = window.URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = downloadUrl;
  link.download = filename;
  document.body.append(link);
  link.click();
  link.remove();
  window.setTimeout(() => {
    window.URL.revokeObjectURL(downloadUrl);
  }, 0);
}

function sortBackups(backups: BackupInfoRecord[]): BackupInfoRecord[] {
  return [...backups].sort((left, right) => {
    const leftTime = Date.parse(left.createdAt);
    const rightTime = Date.parse(right.createdAt);
    if (Number.isFinite(leftTime) && Number.isFinite(rightTime) && leftTime !== rightTime) {
      return rightTime - leftTime;
    }
    return right.filename.localeCompare(left.filename);
  });
}

function upsertBackup(backups: BackupInfoRecord[], nextBackup: BackupInfoRecord): BackupInfoRecord[] {
  return sortBackups([
    nextBackup,
    ...backups.filter((backup) => backup.filename !== nextBackup.filename),
  ]);
}

function normalizeBackupPathDraft(value: string): string | null {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function autoBackupSettingsEqual(
  left: AutoBackupSettings,
  right: AutoBackupSettings,
): boolean {
  return (
    left.enabled === right.enabled &&
    left.dailyTimeLocal === right.dailyTimeLocal &&
    left.autoBackupKeyPresent === right.autoBackupKeyPresent &&
    left.autoBackupDisabledMissingKeyNotice ===
      right.autoBackupDisabledMissingKeyNotice &&
    (left.nextRunAt ?? null) === (right.nextRunAt ?? null)
  );
}

function formatBytes(sizeBytes: number): string {
  const value = sizeBytes;
  if (!Number.isFinite(value) || value < 0) {
    return String(sizeBytes);
  }

  if (value < 1024) {
    return `${value} B`;
  }

  const units = ["KB", "MB", "GB", "TB"];
  let current = value / 1024;
  let unitIndex = 0;
  while (current >= 1024 && unitIndex < units.length - 1) {
    current /= 1024;
    unitIndex += 1;
  }
  return `${current.toFixed(current >= 100 ? 0 : current >= 10 ? 1 : 2)} ${units[unitIndex]}`;
}

function formatDateTime(value: string, dateTimeFormat: UiDateTimeFormat): string {
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) {
    return value;
  }
  return formatUiDateTime(value, dateTimeFormat, { fallback: value });
}

function statusTone(status: BackupInfoRecord["status"]): string {
  switch (status) {
    case "creating":
      return "border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] text-[var(--scry-warning-text)]";
    case "invalid":
    case "failed":
      return "border-destructive/40 bg-destructive/10 text-destructive";
    case "ready":
    default:
      return "border-[var(--scry-success-border)] bg-[var(--scry-success-bg)] text-[var(--scry-success-text)]";
  }
}

function BackupStatusBadge({
  id,
  status,
  label,
}: {
  id?: string;
  status: BackupInfoRecord["status"];
  label: string;
}) {
  return (
    <span
      id={id}
      className={`inline-flex items-center gap-2 rounded-full border px-2.5 py-1 text-xs font-medium ${statusTone(status)}`}
    >
      {status === "creating" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : null}
      {label}
    </span>
  );
}

export function SettingsBackupsContainer() {
  const client = useClient();
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  const setGlobalStatus = useGlobalStatus();

  const [backups, setBackups] = React.useState<BackupInfoRecord[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [savedAutoBackupSettings, setSavedAutoBackupSettings] =
    React.useState<AutoBackupSettings>(DEFAULT_AUTO_BACKUP_SETTINGS);
  const [autoBackupSettings, setAutoBackupSettings] =
    React.useState<AutoBackupSettings>(DEFAULT_AUTO_BACKUP_SETTINGS);
  const [autoBackupLoading, setAutoBackupLoading] = React.useState(true);
  const [autoBackupSaving, setAutoBackupSaving] = React.useState(false);
  const [savedBackupSettings, setSavedBackupSettings] =
    React.useState<BackupSettings>(DEFAULT_BACKUP_SETTINGS);
  const [backupPathDraft, setBackupPathDraft] = React.useState("");
  const [backupSettingsLoading, setBackupSettingsLoading] = React.useState(true);
  const [backupSettingsSaving, setBackupSettingsSaving] = React.useState(false);
  const [folderBrowserOpen, setFolderBrowserOpen] = React.useState(false);
  const [autoBackupExpanded, setAutoBackupExpanded] = React.useState(false);
  const [autoBackupKey, setAutoBackupKey] = React.useState("");
  const [clearAutoBackupKey, setClearAutoBackupKey] = React.useState(false);
  const [showAutoBackupKey, setShowAutoBackupKey] = React.useState(false);
  const [createDialogOpen, setCreateDialogOpen] = React.useState(false);
  const [password, setPassword] = React.useState("");
  const [confirmPassword, setConfirmPassword] = React.useState("");
  const [creatingRequest, setCreatingRequest] = React.useState(false);
  const [selectedBackupFilenames, setSelectedBackupFilenames] = React.useState<Set<string>>(
    () => new Set(),
  );
  const [pendingDeleteFilenames, setPendingDeleteFilenames] = React.useState<string[]>([]);
  const [deletingFilenames, setDeletingFilenames] = React.useState<Set<string>>(() => new Set());
  const [downloadingFilename, setDownloadingFilename] = React.useState<string | null>(null);
  const hasCreatingManualBackup = backups.some(
    (backup) => backup.status === "creating" && backup.trigger === "manual",
  );
  const passwordRequired = password.trim().length === 0;
  const confirmPasswordRequired = confirmPassword.length === 0;
  const passwordMismatch = confirmPassword.length > 0 && password !== confirmPassword;
  const autoBackupReplacementKeyPresent = autoBackupKey.trim().length > 0;
  const autoBackupReplacementKeyTooShort =
    autoBackupReplacementKeyPresent &&
    nonWhitespaceCharacterCount(autoBackupKey) < AUTO_BACKUP_KEY_MIN_LENGTH;
  const autoBackupWillHaveKey =
    !clearAutoBackupKey &&
    (autoBackupReplacementKeyPresent || autoBackupSettings.autoBackupKeyPresent);
  const autoBackupKeyRequired = autoBackupSettings.enabled && !autoBackupWillHaveKey;
  const autoBackupKeyValidationMessage = autoBackupKeyRequired
    ? t("settings.autoBackupsKeyRequired", { count: AUTO_BACKUP_KEY_MIN_LENGTH })
    : autoBackupReplacementKeyTooShort
      ? t("settings.autoBackupsKeyTooShort", { count: AUTO_BACKUP_KEY_MIN_LENGTH })
      : null;
  const canSaveAutoBackupSettings = !autoBackupSaving && !autoBackupKeyValidationMessage;
  const normalizedBackupPathDraft = normalizeBackupPathDraft(backupPathDraft);
  const savedCustomBackupPath = savedBackupSettings.customBackupPath ?? null;
  const backupPathDirty = normalizedBackupPathDraft !== savedCustomBackupPath;
  const canSaveBackupSettings = backupPathDirty && !backupSettingsSaving;
  const canCreateBackup =
    !creatingRequest &&
    !hasCreatingManualBackup &&
    !passwordRequired &&
    !confirmPasswordRequired &&
    !passwordMismatch;
  const pageLoading = loading || autoBackupLoading || backupSettingsLoading;
  const selectableBackups = backups.filter(
    (backup) => backup.status !== "creating" && !deletingFilenames.has(backup.filename),
  );
  const selectedBackups = selectableBackups.filter((backup) =>
    selectedBackupFilenames.has(backup.filename),
  );
  const selectAllState =
    selectedBackups.length === 0
      ? false
      : selectedBackups.length === selectableBackups.length
        ? true
        : "indeterminate";
  const autoBackupNextRunLabel =
    autoBackupSettings.enabled && autoBackupSettings.nextRunAt
      ? formatDateTime(autoBackupSettings.nextRunAt, dateTimeFormat)
      : t("label.disabled");
  const autoBackupKeyPlaceholder = clearAutoBackupKey
    ? ""
    : autoBackupSettings.autoBackupKeyPresent
      ? t("settings.autoBackupsKeyAlreadySetHint")
      : t("settings.autoBackupsSetKey");
  const autoBackupTriggerLabel = (trigger: BackupTrigger) =>
    trigger === "auto" ? t("settings.backupsAutomatic") : t("settings.backupsManual");
  const autoBackupDirty =
    !autoBackupSettingsEqual(autoBackupSettings, savedAutoBackupSettings) ||
    autoBackupKey.length > 0 ||
    clearAutoBackupKey;
  const shouldBlockNavigation =
    (autoBackupDirty || backupPathDirty) && !autoBackupSaving && !backupSettingsSaving;
  const autoBackupNavigationBlocker = useBlocker(shouldBlockNavigation);

  useBeforeUnload(
    React.useCallback((event: BeforeUnloadEvent) => {
      if (!shouldBlockNavigation) {
        return;
      }
      event.preventDefault();
      event.returnValue = "";
    }, [shouldBlockNavigation]),
  );

  const fetchBackups = React.useCallback(async () => {
    try {
      const { data, error } = await client
        .query<BackupsQueryResult>(backupsQuery, {}, { requestPolicy: "network-only" })
        .toPromise();
      if (error) {
        throw error;
      }
      setBackups(sortBackups(data?.backups ?? []));
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToLoad")));
    } finally {
      setLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  const fetchAutoBackupSettings = React.useCallback(async () => {
    try {
      const { data, error } = await client
        .query<AutoBackupSettingsQueryResult>(
          autoBackupSettingsQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      const nextSettings = data?.autoBackupSettings ?? DEFAULT_AUTO_BACKUP_SETTINGS;
      setSavedAutoBackupSettings(nextSettings);
      setAutoBackupSettings(nextSettings);
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToLoad")));
    } finally {
      setAutoBackupLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  const fetchBackupSettings = React.useCallback(async () => {
    try {
      const { data, error } = await client
        .query<BackupSettingsQueryResult>(
          backupSettingsQuery,
          {},
          { requestPolicy: "network-only" },
        )
        .toPromise();
      if (error) {
        throw error;
      }
      const nextSettings = data?.backupSettings ?? DEFAULT_BACKUP_SETTINGS;
      setSavedBackupSettings(nextSettings);
      setBackupPathDraft(nextSettings.customBackupPath ?? "");
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToLoad")));
    } finally {
      setBackupSettingsLoading(false);
    }
  }, [client, setGlobalStatus, t]);

  React.useEffect(() => {
    void fetchBackups();
  }, [fetchBackups]);

  React.useEffect(() => {
    void fetchAutoBackupSettings();
  }, [fetchAutoBackupSettings]);

  React.useEffect(() => {
    void fetchBackupSettings();
  }, [fetchBackupSettings]);

  React.useEffect(() => {
    if (autoBackupSettings.enabled) {
      setAutoBackupExpanded(true);
      return;
    }

    setAutoBackupExpanded(false);
  }, [autoBackupSettings.enabled]);

  useSettingsSubscription(
    React.useCallback(
      (keys: string[]) => {
        if (keys.includes("backup")) {
          void fetchBackups();
        }
        if (keys.includes("backup.path")) {
          void fetchBackupSettings();
          void fetchBackups();
        }
      },
      [fetchBackupSettings, fetchBackups],
    ),
  );

  React.useEffect(() => {
    if (!backups.some((backup) => backup.status === "creating")) {
      return;
    }

    const intervalId = window.setInterval(() => {
      void fetchBackups();
    }, 2000);

    return () => window.clearInterval(intervalId);
  }, [backups, fetchBackups]);

  const handleConfirmLeaveUnsavedBackup = React.useCallback(() => {
    if (autoBackupNavigationBlocker.state !== "blocked") {
      return;
    }
    autoBackupNavigationBlocker.proceed();
  }, [autoBackupNavigationBlocker]);

  const handleCancelLeaveUnsavedBackup = React.useCallback(() => {
    if (autoBackupNavigationBlocker.state !== "blocked") {
      return;
    }
    autoBackupNavigationBlocker.reset();
  }, [autoBackupNavigationBlocker]);

  const handleCreateBackup = React.useCallback(async () => {
    if (!canCreateBackup) {
      return;
    }

    setCreatingRequest(true);
    try {
      const nextPassword = password;
      const { data, error } = await client
        .mutation<CreateBackupMutationResult>(createBackupMutation, {
          input: { password: nextPassword },
        })
        .toPromise();
      if (error || !data?.createBackup) {
        throw error ?? new Error(t("status.failedToUpdate"));
      }

      setBackups((current) => upsertBackup(current, data.createBackup!));
      setPassword("");
      setConfirmPassword("");
      setCreateDialogOpen(false);
      setGlobalStatus(t("settings.backupsQueued"));
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToUpdate")));
    } finally {
      setCreatingRequest(false);
    }
  }, [canCreateBackup, client, password, setGlobalStatus, t]);

  const toggleBackupSelection = React.useCallback(
    (filenames: string[], shouldSelect: boolean) => {
      setSelectedBackupFilenames((current) => {
        const next = new Set(current);
        for (const filename of filenames) {
          if (shouldSelect) {
            next.add(filename);
          } else {
            next.delete(filename);
          }
        }
        return next;
      });
    },
    [],
  );

  const handleDeleteBackups = React.useCallback(async () => {
    if (pendingDeleteFilenames.length === 0) {
      return;
    }

    const filenames = pendingDeleteFilenames;
    const deletedFilenames = new Set<string>();
    const failedFilenames = new Set<string>();
    let firstFailureMessage: string | null = null;
    setDeletingFilenames(new Set(filenames));
    try {
      for (const filename of filenames) {
        try {
          const { data, error } = await client
            .mutation<DeleteBackupMutationResult>(deleteBackupMutation, {
              input: { filename },
            })
            .toPromise();
          if (error || data?.deleteBackup?.deleted !== true) {
            throw error ?? new Error(t("status.failedToDelete"));
          }
          deletedFilenames.add(filename);
        } catch (error) {
          failedFilenames.add(filename);
          firstFailureMessage ??= mutationErrorMessage(error, t("status.failedToDelete"));
        }
      }

      if (deletedFilenames.size > 0) {
        setBackups((current) =>
          current.filter((backup) => !deletedFilenames.has(backup.filename)),
        );
        setSelectedBackupFilenames((current) => {
          const next = new Set(current);
          for (const filename of deletedFilenames) {
            next.delete(filename);
          }
          return next;
        });
      }

      if (failedFilenames.size > 0) {
        setGlobalStatus(
          firstFailureMessage ??
            t("settings.backupsDeleteFailedCount", { count: failedFilenames.size }),
        );
      } else {
        setGlobalStatus(
          t("settings.backupsDeletedCount", { count: deletedFilenames.size }),
        );
      }
    } finally {
      setDeletingFilenames(new Set());
      setPendingDeleteFilenames([]);
    }
  }, [client, pendingDeleteFilenames, setGlobalStatus, t]);

  const handleDownloadBackup = React.useCallback(async (backup: BackupInfoRecord) => {
    setDownloadingFilename(backup.filename);
    try {
      const { data, error } = await client
        .mutation<PrepareBackupDownloadMutationResult>(prepareBackupDownloadMutation, {
          input: { filename: backup.filename },
        })
        .toPromise();
      const downloadUrl = data?.prepareBackupDownload?.downloadUrl;
      const downloadAuthorizationToken = data?.prepareBackupDownload?.downloadAuthorizationToken;
      if (error || !downloadUrl || !downloadAuthorizationToken) {
        throw error ?? new Error(t("status.failedToLoad"));
      }

      const response = await scryerFetch(buildAppUrl(downloadUrl), {
        headers: {
          Authorization: `Bearer ${downloadAuthorizationToken}`,
        },
      });
      if (!response.ok) {
        throw new Error(
          await readResponseErrorMessage(response, t("status.failedToLoad")),
        );
      }

      await saveDownloadResponse(response, backup.filename);
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToLoad")));
    } finally {
      setDownloadingFilename(null);
    }
  }, [client, setGlobalStatus, t]);

  const handleSaveAutoBackupSettings = React.useCallback(async () => {
    if (autoBackupKeyValidationMessage) {
      setGlobalStatus(autoBackupKeyValidationMessage);
      return;
    }

    setAutoBackupSaving(true);
    try {
      const nextAutoBackupKey =
        clearAutoBackupKey ? null : autoBackupReplacementKeyPresent ? autoBackupKey : null;
      const { data, error } = await client
        .mutation<UpdateAutoBackupSettingsMutationResult>(updateAutoBackupSettingsMutation, {
          input: {
            enabled: autoBackupSettings.enabled,
            dailyTimeLocal: autoBackupSettings.dailyTimeLocal,
            setAutoBackupKey: nextAutoBackupKey,
            clearAutoBackupKey,
          },
        })
        .toPromise();
      if (error || !data?.updateAutoBackupSettings) {
        throw error ?? new Error(t("status.failedToUpdate"));
      }

      setSavedAutoBackupSettings(data.updateAutoBackupSettings);
      setAutoBackupSettings(data.updateAutoBackupSettings);
      setAutoBackupKey("");
      setClearAutoBackupKey(false);
      setShowAutoBackupKey(false);
      setGlobalStatus(t("settings.autoBackupsSaved"));
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToUpdate")));
    } finally {
      setAutoBackupSaving(false);
    }
  }, [
    autoBackupKey,
    autoBackupKeyValidationMessage,
    autoBackupReplacementKeyPresent,
    autoBackupSettings.dailyTimeLocal,
    autoBackupSettings.enabled,
    clearAutoBackupKey,
    client,
    setGlobalStatus,
    t,
  ]);

  const handleSaveBackupSettings = React.useCallback(async () => {
    setBackupSettingsSaving(true);
    try {
      const customBackupPath = normalizeBackupPathDraft(backupPathDraft);
      const { data, error } = await client
        .mutation<UpdateBackupSettingsMutationResult>(updateBackupSettingsMutation, {
          input: { customBackupPath },
        })
        .toPromise();
      if (error || !data?.updateBackupSettings) {
        throw error ?? new Error(t("status.failedToUpdate"));
      }

      setSavedBackupSettings(data.updateBackupSettings);
      setBackupPathDraft(data.updateBackupSettings.customBackupPath ?? "");
      await fetchBackups();
      setGlobalStatus(t("settings.backupLocationSaved"));
    } catch (error) {
      setGlobalStatus(mutationErrorMessage(error, t("status.failedToUpdate")));
    } finally {
      setBackupSettingsSaving(false);
    }
  }, [backupPathDraft, client, fetchBackups, setGlobalStatus, t]);

  const handleResetBackupPath = React.useCallback(() => {
    setBackupPathDraft("");
  }, []);

  if (pageLoading) {
    return (
      <div className={`flex items-center gap-2 text-sm ${BACKUPS_MUTED_TEXT_CLASS}`}>
        <Loader2 className="h-4 w-4 animate-spin" />
        {t("label.loading")}
      </div>
    );
  }

  return (
    <>
      <div className="space-y-4 text-sm">
        <section className={BACKUPS_PANEL_CLASS}>
          <div className={BACKUPS_PANEL_HEADER_CLASS}>
            <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
              <button
                type="button"
                className="flex min-w-0 flex-1 items-start gap-3 text-left"
                onClick={() => setAutoBackupExpanded((current) => !current)}
                aria-expanded={autoBackupExpanded}
              >
                <div className="min-w-0 flex-1 space-y-1">
                  <h2 className={BACKUPS_PANEL_TITLE_CLASS}>
                    {t("settings.autoBackupsTitle")}
                  </h2>
                  <p className={`text-sm ${BACKUPS_MUTED_TEXT_CLASS}`}>
                    {t("settings.autoBackupsDescription")}
                  </p>
                </div>
              </button>
              <div className="flex shrink-0 justify-end sm:pt-1">
                <SettingsToggleSwitch
                  checked={autoBackupSettings.enabled}
                  disabled={autoBackupSaving}
                  size="lg"
                  ariaLabel={
                    autoBackupSettings.enabled
                      ? t("label.enabled")
                      : t("label.disabled")
                  }
                  onChange={(nextValue) =>
                    setAutoBackupSettings((current) => ({
                      ...current,
                      enabled: nextValue,
                    }))
                  }
                />
              </div>
            </div>
          </div>
          {autoBackupExpanded ? (
            <div className={`${BACKUPS_PANEL_BODY_CLASS} space-y-4`}>
              <div className="grid gap-4 md:grid-cols-2 md:items-stretch">
                <div className={`flex h-full min-h-32 flex-col p-4 ${BACKUPS_INSET_CLASS}`}>
                  <Label
                    htmlFor="auto-backups-time"
                    className="text-sm font-medium text-[var(--scry-ink2)]"
                  >
                    {t("settings.autoBackupsTime")}
                  </Label>
                  <div className="mt-3">
                    <TimePicker
                      id="auto-backups-time"
                      value={autoBackupSettings.dailyTimeLocal}
                      disabled={autoBackupSaving}
                      hourLabel={t("settings.autoBackupsHour")}
                      minuteLabel={t("settings.autoBackupsMinute")}
                      onChange={(nextValue) =>
                        setAutoBackupSettings((current) => ({
                          ...current,
                          dailyTimeLocal: nextValue,
                        }))
                      }
                    />
                  </div>
                  <p className={`mt-auto pt-4 text-xs ${BACKUPS_MUTED_TEXT_CLASS}`}>
                    {t("settings.autoBackupsTimeHelp")}
                  </p>
                </div>

                <div className={`flex h-full min-h-32 flex-col p-4 ${BACKUPS_INSET_CLASS}`}>
                  <p className="text-sm font-medium text-[var(--scry-ink2)]">
                    {t("settings.autoBackupsNextRun")}
                  </p>
                  <p className="mt-3 text-base font-medium text-[var(--scry-ink2)]">
                    {autoBackupNextRunLabel}
                  </p>
                  <p className={`mt-auto pt-4 text-xs ${BACKUPS_MUTED_TEXT_CLASS}`}>
                    {t("settings.autoBackupsNextRunHelp")}
                  </p>
                </div>
              </div>

              <div className="space-y-2 text-sm">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-[var(--scry-ink2)]">
                    {t("settings.autoBackupsKeyLabel")}
                  </span>
                  <InfoHelp
                    text={t("settings.autoBackupsKeyHelp", {
                      count: AUTO_BACKUP_KEY_MIN_LENGTH,
                    })}
                    ariaLabel={t("settings.autoBackupsKeyHelpLabel")}
                  />
                </div>
                <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
                  <div className="w-full max-w-sm space-y-2">
                    <div className="relative">
                      <Input
                        type={showAutoBackupKey ? "text" : "password"}
                        value={autoBackupKey}
                        placeholder={autoBackupKeyPlaceholder}
                        className="pr-11"
                        disabled={autoBackupSaving || clearAutoBackupKey}
                        onChange={(event) => {
                          const nextValue = event.target.value;
                          setAutoBackupKey(nextValue);
                          if (nextValue.trim().length > 0) {
                            setClearAutoBackupKey(false);
                          }
                        }}
                      />
                      <button
                        type="button"
                        className="absolute inset-y-0 right-0 flex w-10 items-center justify-center text-muted-foreground transition hover:text-foreground disabled:cursor-not-allowed disabled:opacity-50"
                        aria-label={
                          showAutoBackupKey
                            ? t("settings.autoBackupsHideKey")
                            : t("settings.autoBackupsShowKey")
                        }
                        disabled={autoBackupSaving || clearAutoBackupKey}
                        onClick={() => setShowAutoBackupKey((current) => !current)}
                      >
                        {showAutoBackupKey ? (
                          <EyeOff className="h-4 w-4" />
                        ) : (
                          <Eye className="h-4 w-4" />
                        )}
                      </button>
                    </div>
                    {autoBackupKeyValidationMessage ? (
                      <p className="text-xs text-destructive">
                        {autoBackupKeyValidationMessage}
                      </p>
                    ) : null}
                  </div>

                  <div className="flex shrink-0 flex-wrap items-center gap-3 sm:min-w-64">
                    <Button
                      type="button"
                      variant="primary"
                      onClick={() => void handleSaveAutoBackupSettings()}
                      disabled={!canSaveAutoBackupSettings}
                    >
                      {autoBackupSaving ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                      {t("label.save")}
                    </Button>

                    {autoBackupSettings.autoBackupKeyPresent ? (
                      <div className={`flex min-w-0 items-center gap-3 px-4 py-3 sm:max-w-xl ${BACKUPS_INSET_CLASS}`}>
                        <Checkbox
                          id="auto-backups-clear-key"
                          className="size-5 rounded-md"
                          checked={clearAutoBackupKey}
                          disabled={
                            autoBackupSaving || autoBackupSettings.enabled || autoBackupKey.length > 0
                          }
                          onCheckedChange={(checked) => {
                            const shouldClear = checked === true;
                            setClearAutoBackupKey(shouldClear);
                            if (shouldClear) {
                              setAutoBackupKey("");
                              setShowAutoBackupKey(false);
                            }
                          }}
                        />
                        <div className="flex min-w-0 items-center gap-2">
                          <Label
                            htmlFor="auto-backups-clear-key"
                            className="truncate text-[var(--scry-ink2)]"
                          >
                            {t("settings.autoBackupsClearKey")}
                          </Label>
                          <InfoHelp
                            text={t("settings.autoBackupsClearKeyHelp")}
                            ariaLabel={t("settings.autoBackupsClearKey")}
                          />
                        </div>
                      </div>
                    ) : null}
                  </div>
                </div>
              </div>
            </div>
          ) : null}
        </section>

        <section className={BACKUPS_PANEL_CLASS}>
          <div className={BACKUPS_PANEL_HEADER_CLASS}>
            <h2 className={BACKUPS_PANEL_TITLE_CLASS}>
              {t("settings.backupLocationTitle")}
            </h2>
            <p className={`mt-1 text-sm ${BACKUPS_MUTED_TEXT_CLASS}`}>
              {t("settings.backupLocationDescription")}
            </p>
          </div>
          <div className={`${BACKUPS_PANEL_BODY_CLASS} space-y-4`}>
            <div className="grid gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(18rem,0.75fr)]">
              <div className="space-y-2">
                <Label htmlFor="backup-location-path" className="text-[var(--scry-ink2)]">
                  {t("settings.backupLocationCustomPath")}
                </Label>
                <div className="flex flex-col gap-2 sm:flex-row">
                  <Input
                    id="backup-location-path"
                    value={backupPathDraft}
                    placeholder={savedBackupSettings.defaultBackupPath}
                    disabled={backupSettingsSaving}
                    onChange={(event) => setBackupPathDraft(event.target.value)}
                    className="font-[var(--font-code)]"
                  />
                  <Button
                    id="backup-location-browse"
                    type="button"
                    variant="outline"
                    className="shrink-0"
                    disabled={backupSettingsSaving}
                    onClick={() => setFolderBrowserOpen(true)}
                  >
                    <FolderOpen className="h-4 w-4" />
                    {t("settings.backupLocationBrowse")}
                  </Button>
                </div>
              </div>

              <div className={`space-y-2 p-4 ${BACKUPS_INSET_CLASS}`}>
                <p className={`text-xs font-medium uppercase ${BACKUPS_MUTED_TEXT_CLASS}`}>
                  {t("settings.backupLocationEffectivePath")}
                </p>
                <p
                  id="backup-location-effective-path"
                  className="break-all font-[var(--font-code)] text-sm text-[var(--scry-ink2)]"
                >
                  {savedBackupSettings.effectiveBackupPath || savedBackupSettings.defaultBackupPath}
                </p>
              </div>
            </div>

            <p className={`text-xs ${BACKUPS_MUTED_TEXT_CLASS}`}>
              {t("settings.backupLocationHelp")}
            </p>

            <div className="flex flex-col gap-2 sm:flex-row sm:items-center">
              <Button
                id="backup-location-save"
                type="button"
                variant="primary"
                onClick={() => void handleSaveBackupSettings()}
                disabled={!canSaveBackupSettings}
              >
                {backupSettingsSaving ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
                {t("label.save")}
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={handleResetBackupPath}
                disabled={backupSettingsSaving || backupPathDraft.trim().length === 0}
              >
                <RotateCcw className="h-4 w-4" />
                {t("settings.backupLocationReset")}
              </Button>
            </div>
          </div>
        </section>

        <section className={BACKUPS_PANEL_CLASS}>
          <div className={BACKUPS_PANEL_HEADER_CLASS}>
            <div className="grid grid-cols-[1fr_auto_1fr] items-center">
              {selectedBackups.length > 0 ? (
                <div className="col-start-2">
                  <Button
                    id={selectorId("settings-backups-delete-selected")}
                    type="button"
                    variant="outline"
                    className="text-[var(--scry-danger-text-soft)] hover:text-[var(--scry-danger-text)]"
                    onClick={() =>
                      setPendingDeleteFilenames(selectedBackups.map((backup) => backup.filename))
                    }
                  >
                    <Trash2 className="h-4 w-4" />
                    {t("settings.backupsDeleteSelected")}
                  </Button>
                </div>
              ) : null}
              <Button
                id={selectorId("settings-backups-create-open")}
                type="button"
                variant="primary"
                className="col-start-3 shrink-0 justify-self-end"
                onClick={() => setCreateDialogOpen(true)}
                disabled={hasCreatingManualBackup}
              >
                <Plus className="h-4 w-4" />
                {t("settings.backupsCreate")}
              </Button>
            </div>
          </div>
          {backups.length === 0 ? (
            <div className={`${BACKUPS_PANEL_BODY_CLASS}`}>
              <div className={`rounded-[12px] border border-dashed border-[var(--scry-border3)] px-4 py-8 text-center text-sm ${BACKUPS_MUTED_TEXT_CLASS}`}>
                {t("settings.backupsEmpty")}
              </div>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow className="border-[var(--scry-border3)] bg-[var(--scry-inset)] hover:bg-[var(--scry-inset)]">
                    <TableCheckboxHead>
                      <Checkbox
                        id="settings-backups-select-all"
                        size="table"
                        checked={selectAllState}
                        disabled={selectableBackups.length === 0}
                        aria-label={t("settings.backupsSelectAll")}
                        onCheckedChange={(checked) =>
                          toggleBackupSelection(
                            selectableBackups.map((backup) => backup.filename),
                            checked === true,
                          )
                        }
                      />
                    </TableCheckboxHead>
                    <TableHead className={`font-semibold ${BACKUPS_MUTED_TEXT_CLASS}`}>Bundle</TableHead>
                    <TableHead className={`font-semibold ${BACKUPS_MUTED_TEXT_CLASS}`}>Created</TableHead>
                    <TableHead className={`font-semibold ${BACKUPS_MUTED_TEXT_CLASS}`}>
                      {t("label.status")}
                    </TableHead>
                    <TableHead className={`font-semibold ${BACKUPS_MUTED_TEXT_CLASS}`}>Size</TableHead>
                    <TableHead className={`text-right font-semibold ${BACKUPS_MUTED_TEXT_CLASS}`}>
                      {t("label.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {backups.map((backup) => {
                    const isDeleting = deletingFilenames.has(backup.filename);
                    const isDownloading = downloadingFilename === backup.filename;
                    const disableActions = backup.status === "creating" || isDeleting;
                    const statusLabel =
                      backup.status === "creating"
                        ? t("settings.backupsCreating")
                        : backup.status === "invalid"
                          ? t("settings.backupsInvalid")
                        : backup.status === "failed"
                          ? t("settings.backupsFailed")
                          : t("settings.backupsReady");

                    return (
                      <TableRow
                        data-ui="settings-table-row"
                        key={backup.filename}
                        id={selectorId("settings-backup-row", "created-at", backup.createdAt)}
                        className="border-[var(--scry-border3)] hover:bg-[var(--scry-rowHover)]"
                      >
                        <TableCheckboxCell>
                          <Checkbox
                            id={selectorId("settings-backup-select", "created-at", backup.createdAt)}
                            size="table"
                            checked={selectedBackupFilenames.has(backup.filename)}
                            disabled={disableActions}
                            aria-label={t("settings.backupsSelect", { name: backup.filename })}
                            onCheckedChange={(checked) =>
                              toggleBackupSelection([backup.filename], checked === true)
                            }
                          />
                        </TableCheckboxCell>
                        <TableCell className="align-top">
                          <div className="space-y-1">
                            <div
                              id={selectorId(
                                "settings-backup-filename",
                                "created-at",
                                backup.createdAt,
                              )}
                              className="font-medium text-[var(--scry-ink2)]"
                            >
                              {backup.filename}
                            </div>
                            <div className={`flex flex-wrap items-center gap-2 text-xs ${BACKUPS_MUTED_TEXT_CLASS}`}>
                              <span className="rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-0.5">
                                {backup.encrypted
                                  ? t("settings.backupsEncrypted")
                                  : t("settings.backupsPlaintext")}
                              </span>
                              <span className="rounded-full border border-[var(--scry-border3)] bg-[var(--scry-inset)] px-2 py-0.5">
                                {autoBackupTriggerLabel(backup.trigger)}
                              </span>
                              <span>{backup.formatVersion}</span>
                              <span>{backup.sourceEngine}</span>
                              {backup.sourceMigrationKey ? <span>{backup.sourceMigrationKey}</span> : null}
                            </div>
                            {backup.errorMessage ? (
                              <p className="text-xs text-destructive">{backup.errorMessage}</p>
                            ) : null}
                          </div>
                        </TableCell>
                        <TableCell className={`whitespace-nowrap ${BACKUPS_MUTED_TEXT_CLASS}`}>
                          {formatDateTime(backup.createdAt, dateTimeFormat)}
                        </TableCell>
                        <TableCell>
                          <BackupStatusBadge
                            id={selectorId(
                              "settings-backup-status",
                              backup.status,
                              "created-at",
                              backup.createdAt,
                            )}
                            status={backup.status}
                            label={statusLabel}
                          />
                        </TableCell>
                        <TableCell className={`align-middle font-[var(--font-code)] text-xs ${BACKUPS_MUTED_TEXT_CLASS}`}>
                          {formatBytes(backup.sizeBytes)}
                        </TableCell>
                        <TableCell>
                          <div className="flex items-center justify-end gap-1">
                            {backup.status === "ready" ? (
                              <IconButton
                                id={selectorId(
                                  "settings-backup-download",
                                  "created-at",
                                  backup.createdAt,
                                )}
                                label={t("settings.backupsDownload")}
                                tone="install"
                                disabled={isDownloading || isDeleting}
                                onClick={() => void handleDownloadBackup(backup)}
                              >
                                {isDownloading ? (
                                  <Loader2 className="h-4 w-4 animate-spin" />
                                ) : (
                                  <Download className="h-4 w-4" />
                                )}
                              </IconButton>
                            ) : null}
                            <IconButton
                              id={selectorId("settings-backup-delete", "created-at", backup.createdAt)}
                              label={t("settings.backupsDelete")}
                              tone="delete"
                              disabled={disableActions}
                              onClick={() => setPendingDeleteFilenames([backup.filename])}
                            >
                              {isDeleting ? (
                                <Loader2 className="h-4 w-4 animate-spin" />
                              ) : (
                                <Trash2 className="h-4 w-4" />
                              )}
                            </IconButton>
                          </div>
                        </TableCell>
                      </TableRow>
                    );
                  })}
                </TableBody>
              </Table>
            </div>
          )}
        </section>
      </div>

      <FolderBrowserDialog
        open={folderBrowserOpen}
        onOpenChange={setFolderBrowserOpen}
        selectionTypes={["folder"]}
        initialPath={
          backupPathDraft.trim() ||
          savedBackupSettings.effectiveBackupPath ||
          savedBackupSettings.defaultBackupPath ||
          "/"
        }
        title={t("settings.backupLocationPickerTitle")}
        onSelect={(path) => {
          setBackupPathDraft(path);
          setFolderBrowserOpen(false);
        }}
      />

      <Dialog
        open={createDialogOpen}
        onOpenChange={(open) => {
          setCreateDialogOpen(open);
          if (!open) {
            setPassword("");
            setConfirmPassword("");
          }
        }}
      >
        <DialogContent id={selectorId("settings-backups-create-dialog")} className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{t("settings.backupsCreateTitle")}</DialogTitle>
            <DialogDescription>{t("settings.backupsCreateDescription")}</DialogDescription>
          </DialogHeader>
          <div className="space-y-3">
            <label className="block space-y-2 text-sm">
              <span className="font-medium">{t("settings.password")}</span>
              <Input
                id={selectorId("settings-backups-create-password")}
                type="password"
                value={password}
                onChange={(event) => {
                  const nextPassword = event.target.value;
                  setPassword(nextPassword);
                  if (nextPassword.length === 0) {
                    setConfirmPassword("");
                  }
                }}
                placeholder={t("settings.password")}
                disabled={creatingRequest}
                required
              />
            </label>
            <label className="block space-y-2 text-sm">
              <span className="font-medium">{t("settings.backupsConfirmPassword")}</span>
              <Input
                id={selectorId("settings-backups-create-confirm-password")}
                type="password"
                value={confirmPassword}
                onChange={(event) => setConfirmPassword(event.target.value)}
                placeholder={t("settings.backupsConfirmPassword")}
                disabled={creatingRequest}
                required
              />
              {passwordMismatch ? (
                <p className="text-xs text-destructive">
                  {t("settings.backupsPasswordMismatch")}
                </p>
              ) : null}
            </label>
            <div className="rounded-lg border border-border bg-muted/30 p-3 text-xs text-muted-foreground">
              <div className="mb-1 flex items-center gap-2 text-foreground">
                <LockKeyhole className="h-3.5 w-3.5" />
                <span>{t("settings.backupsRequiredPassword")}</span>
              </div>
              <p>{t("settings.backupsPasswordHelp")}</p>
            </div>
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="secondary"
              onClick={() => setCreateDialogOpen(false)}
              disabled={creatingRequest}
            >
              {t("label.cancel")}
            </Button>
            <Button
              id={selectorId("settings-backups-create-submit")}
              type="button"
              onClick={() => void handleCreateBackup()}
              disabled={!canCreateBackup}
            >
              {creatingRequest ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {t("settings.backupsCreate")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <ConfirmDialog
        open={pendingDeleteFilenames.length > 0}
        title={t("settings.backupsDelete")}
        description={
          pendingDeleteFilenames.length === 1
            ? t("settings.backupsDeleteConfirm")
            : t("settings.backupsDeleteSelectedConfirm", {
                count: pendingDeleteFilenames.length,
              })
        }
        confirmLabel={t("settings.backupsDelete")}
        cancelLabel={t("label.cancel")}
        isBusy={deletingFilenames.size > 0}
        onConfirm={handleDeleteBackups}
        onCancel={() => {
          if (deletingFilenames.size > 0) {
            return;
          }
          setPendingDeleteFilenames([]);
        }}
      />
      <ConfirmDialog
        open={autoBackupNavigationBlocker.state === "blocked"}
        title={t("settings.unsavedBackupChangesTitle")}
        description={t("settings.unsavedBackupChangesConfirm")}
        confirmLabel={t("label.discard")}
        cancelLabel={t("label.cancel")}
        onConfirm={handleConfirmLeaveUnsavedBackup}
        onCancel={handleCancelLeaveUnsavedBackup}
      />
    </>
  );
}
