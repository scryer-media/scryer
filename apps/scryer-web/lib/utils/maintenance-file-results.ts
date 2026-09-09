export type MaintenanceFileResult = {
  fileId: string;
  filePath: string | null;
  completed: boolean;
  error: string | null;
};

export type MaintenanceFileResults = {
  files: MaintenanceFileResult[];
  totalSizeBytes: number | null;
  graceDeadline: string | null;
  storageRootId: string | null;
  completedFileCount: number;
  remainingFileCount: number;
};

/// Read the versioned scoped-deletion checkpoint embedded in an action-run
/// detail payload. Older checkpoints omit `storage_root_id`; they retain their
/// original file and grace-deadline rendering with a null root.
export function parseMaintenanceFileResults(detail: string): MaintenanceFileResults | null {
  let value: unknown;
  try {
    value = JSON.parse(detail);
  } catch {
    return null;
  }
  if (!isRecord(value) || !Array.isArray(value.files)) {
    return null;
  }
  const files = value.files.flatMap((file): MaintenanceFileResult[] => {
    if (
      !isRecord(file) ||
      typeof file.file_id !== "string" ||
      typeof file.completed !== "boolean"
    ) {
      return [];
    }
    return [
      {
        fileId: file.file_id,
        filePath: typeof file.file_path === "string" ? file.file_path : null,
        completed: file.completed,
        error: typeof file.error === "string" ? file.error : null,
      },
    ];
  });
  if (files.length === 0) {
    return null;
  }
  const completedFileCount = files.filter((file) => file.completed).length;
  return {
    files,
    totalSizeBytes:
      typeof value.total_size_bytes === "number" && Number.isFinite(value.total_size_bytes)
        ? value.total_size_bytes
        : null,
    graceDeadline: typeof value.grace_deadline === "string" ? value.grace_deadline : null,
    storageRootId: typeof value.storage_root_id === "string" ? value.storage_root_id : null,
    completedFileCount,
    remainingFileCount: files.length - completedFileCount,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
