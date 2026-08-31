import type {
  ApplicationUpgradePhase,
  ApplicationUpgradeProgress,
} from "@/lib/types";

const KNOWN_PHASES = new Set<ApplicationUpgradePhase>([
  "checking",
  "downloading",
  "verifying",
  "staging",
  "applying",
  "awaiting_elevation",
  "restarting",
  "reboot_required",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseProgressValue(value: unknown): Record<string, unknown> | null {
  if (isRecord(value)) {
    return value;
  }
  if (typeof value !== "string") {
    return null;
  }

  try {
    const parsed: unknown = JSON.parse(value);
    return isRecord(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

function nullableText(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function nullableByteCount(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? value
    : null;
}

export function normalizeApplicationUpgradeProgress(
  value: unknown,
): ApplicationUpgradeProgress | null {
  const progress = parseProgressValue(value);
  if (!progress) {
    return null;
  }

  const rawPhase = nullableText(progress.phase);
  return {
    status: nullableText(progress.status),
    phase: rawPhase ? (KNOWN_PHASES.has(rawPhase as ApplicationUpgradePhase) ? rawPhase as ApplicationUpgradePhase : "unknown") : null,
    downloadedBytes: nullableByteCount(progress.downloadedBytes),
    totalBytes: nullableByteCount(progress.totalBytes),
    targetVersion: nullableText(progress.targetVersion),
    targetTag: nullableText(progress.targetTag),
    error: nullableText(progress.error),
  };
}
