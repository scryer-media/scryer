import type { Translate } from "@/components/root/types";
import type { JobRun } from "@/lib/types";
import {
  graphQlErrorExtension,
  userFacingGraphQlErrorMessage,
} from "../graphql/error-message.ts";

export const SUBTITLE_BATCH_REFUSAL_KEYS: Readonly<Record<string, string>> = {
  SUBTITLES_BATCH_SELECTION_INVALID: "subtitle.batchSelectionInvalid",
  SUBTITLES_BATCH_DISABLED: "subtitle.subtitlesDisabledBody",
  SUBTITLES_BATCH_LANGUAGES_REQUIRED: "subtitle.batchLanguagesRequired",
  SUBTITLES_BATCH_ALREADY_RUNNING: "subtitle.batchAlreadyRunning",
};

export function subtitleBatchErrorMessage(
  error: unknown,
  t: Translate,
): string {
  const reason = graphQlErrorExtension(error, "VALIDATION_ERROR", "reason");
  const key = reason && SUBTITLE_BATCH_REFUSAL_KEYS[reason];
  return key
    ? t(key)
    : userFacingGraphQlErrorMessage(error, t("subtitle.batchFailed"), t);
}

export function episodeSelectionActions(
  canDelete: boolean,
  canSearchSubtitles: boolean,
) {
  return {
    enabled: canDelete || canSearchSubtitles,
    canDelete,
    canSearchSubtitles,
  };
}

/** Only an accepted tracked job clears the selection. Provider work runs on the server. */
export async function submitEpisodeSubtitleBatch(
  request: () => Promise<JobRun>,
  register: (run: JobRun) => void,
  clear: () => void,
): Promise<void> {
  const run = await request();
  register(run);
  clear();
}

function progressObject(value: unknown): Record<string, unknown> {
  try {
    const parsed: unknown =
      typeof value === "string" ? JSON.parse(value) : value;
    return typeof parsed === "object" &&
      parsed !== null &&
      !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

export function isEpisodeSubtitleBatch(
  run: Pick<JobRun, "jobKey" | "progressJson" | "summaryJson">,
): boolean {
  return (
    run.jobKey === "SUBTITLE_SEARCH" &&
    (progressObject(run.progressJson).kind === "episode_subtitle_search" ||
      progressObject(run.summaryJson).kind === "episode_subtitle_search")
  );
}

export function subtitleBatchProgressText(
  run: JobRun,
  t: Translate,
): string | null {
  if (!isEpisodeSubtitleBatch(run)) return null;
  const progress = progressObject(
    run.completedAt ? run.summaryJson : run.progressJson,
  );
  const number = (key: string) =>
    typeof progress[key] === "number" && Number.isFinite(progress[key])
      ? progress[key]
      : 0;
  if (run.status === "FAILED") return t("subtitle.batchRunFailed");
  if (run.completedAt)
    return t("subtitle.batchCompleted", {
      downloaded: number("downloaded"),
      warnings: number("warnings"),
    });
  return t("subtitle.batchProgress", {
    completed: number("completed"),
    total: number("total"),
  });
}

export function completedSubtitleBatchIds(runs: JobRun[]): Set<string> {
  return new Set(
    runs
      .filter((run) => isEpisodeSubtitleBatch(run) && run.completedAt)
      .map((run) => run.id),
  );
}
