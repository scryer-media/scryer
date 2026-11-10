type Translate = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => string;

type JobRunName = { jobKey: string; displayName: string };

/** Display name supplied to job-run notifications. */
export function jobRunDisplayName(run: JobRunName, t: Translate): string {
  return builtinText({ key: run.jobKey }, "name", run.displayName, t);
}

type NativeJob = {
  key: string;
  displayName: string;
  description: string;
  schedule: { description: string };
};

/**
 * Schedule wording the backend fills in per job. The backend sends no code
 * for these, so the fixed sentences are looked up here; anything else is
 * shown as sent.
 */
const scheduleTextKeys: Record<string, string> = {
  "Manual only": "jobs.builtinText.Manual_only",
  "Every 2 hours": "jobs.builtinText.Every_2_hours",
  "Every 6 hours": "jobs.builtinText.Every_6_hours",
  "Every 8 hours": "jobs.builtinText.Every_8_hours",
  "Every 12 hours": "jobs.builtinText.Every_12_hours",
  "Every 24 hours": "jobs.builtinText.Every_24_hours",
  "Every 15 minutes": "jobs.builtinText.Every_15_minutes",
  "Every hour": "jobs.builtinText.Every_hour",
  "Daily at configured local time":
    "jobs.builtinText.Daily_at_configured_local_time",
  "Dynamic discovery evaluator with daily backstop":
    "jobs.builtinText.Dynamic_discovery_evaluator_with_daily_backstop",
  "Every 30 minutes, 02:00–06:00 host local time":
    "jobs.builtinText.Every_30_minutes_02_00_06_00_host_local_time",
  "Re-evaluated during RSS sync":
    "jobs.builtinText.Re_evaluated_during_RSS_sync",
  "Scheduler-paced per indexer (15-minute target)":
    "jobs.builtinText.Scheduler_paced_per_indexer_15_minute_target",
  "Based on subtitle settings interval":
    "jobs.builtinText.Based_on_subtitle_settings_interval",
};

/** Artwork encoding's description carries its current wait state. */
const artworkDescriptionKeys: Record<string, string> = {
  "Encode pending artwork. Run now overrides the nightly window. Waiting for pending artwork.":
    "jobs.builtinText.Encode_pending_artwork_Run_now_overrides_the_nightly_window_Waiting_for_pending_artwork",
  "Encode pending artwork. Run now overrides the nightly window. Waiting for the nightly window.":
    "jobs.artworkWait.nightly",
  "Encode pending artwork. Run now overrides the nightly window. Waiting for library scans.":
    "jobs.artworkWait.scan",
};

const ARTWORK_WINDOW =
  /^Daily (\d{2}:\d{2})–(\d{2}:\d{2}) host local time; host now (.+)$/;

/** `LIBRARY_SCAN_MOVIES` → `LibraryScanMovies`, the name the keys use. */
function jobKind(key: string): string {
  return key
    .toLowerCase()
    .split("_")
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join("");
}

function builtinText(
  job: Pick<NativeJob, "key">,
  field: "name" | "description",
  fallback: string,
  t: Translate,
): string {
  if (job.key === "CUSTOM_JOB") return fallback;
  const key = `jobs.builtin.${jobKind(job.key)}.${field}`;
  const translated = t(key);
  return translated === key ? fallback : translated;
}

/** A built-in job's name in the UI language; a user script's name as written. */
export function jobDisplayName(job: NativeJob, t: Translate): string {
  return builtinText(job, "name", job.displayName, t);
}

/** A built-in job's description in the UI language. */
export function jobDescription(job: NativeJob, t: Translate): string {
  if (job.key === "ARTWORK_ENCODING") {
    const key = artworkDescriptionKeys[job.description];
    return key ? t(key) : job.description;
  }
  return builtinText(job, "description", job.description, t);
}

/** A built-in job's schedule in the UI language; unknown wording as sent. */
export function jobScheduleDescription(job: NativeJob, t: Translate): string {
  const text = job.schedule.description;
  if (job.key === "CUSTOM_JOB") return text;
  const window = ARTWORK_WINDOW.exec(text);
  if (window) {
    return t("jobs.artworkWindow", {
      start: window[1],
      end: window[2],
      clock: window[3],
    });
  }
  const key = scheduleTextKeys[text];
  return key ? t(key) : text;
}
