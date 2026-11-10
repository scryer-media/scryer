import assert from "node:assert/strict";
import test from "node:test";
import {
  jobDescription,
  jobDisplayName,
  jobScheduleDescription,
} from "./native-jobs.ts";
import ru from "./locales/ru.ts";
import { interpolate } from "./types.ts";
const t = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => interpolate(ru[key] ?? key, values);

function job(
  key: string,
  displayName: string,
  description = "",
  schedule = "",
) {
  return { key, displayName, description, schedule: { description: schedule } };
}

test("built-in job names follow the job key, not the backend's wording", () => {
  assert.equal(
    jobDisplayName(job("ARTWORK_ENCODING", "Artwork Encoding"), t),
    "Кодирование изображений",
  );
  assert.equal(
    jobDisplayName(
      job("LIBRARY_SCAN_MOVIES", "Movie library scan (renamed)"),
      t,
    ),
    ru["jobs.builtin.LibraryScanMovies.name"],
  );
  assert.equal(
    jobDescription(job("RSS_SYNC", "RSS Sync", "Reworded upstream."), t),
    ru["jobs.builtin.RssSync.description"],
  );
});

test("artwork window labels localize without losing the host clock or timezone", () => {
  assert.equal(
    jobScheduleDescription(
      job(
        "ARTWORK_ENCODING",
        "Artwork Encoding",
        "",
        "Daily 03:00–09:00 host local time; host now 2026-10-31 08:31 (UTC+00:00)",
      ),
      t,
    ),
    "Ежедневно с 03:00 до 09:00 по местному времени сервера; сейчас на сервере 2026-10-31 08:31 (UTC+00:00)",
  );
});

test("nightly artwork wait states keep their meaning in Russian", () => {
  for (const [reason, expected] of [
    ["Waiting for the nightly window", "Ожидание ночного окна."],
    [
      "Waiting for library scans",
      "Ожидание завершения сканирования библиотек.",
    ],
  ]) {
    const output = jobDescription(
      job(
        "ARTWORK_ENCODING",
        "Artwork Encoding",
        `Encode pending artwork. Run now overrides the nightly window. ${reason}.`,
      ),
      t,
    );
    assert.ok(output.endsWith(expected), output);
  }
});

test("user-defined jobs and unknown jobs keep the text they were sent", () => {
  const script = job(
    "CUSTOM_JOB",
    "Artwork Encoding",
    "Housekeeping",
    "Manual only",
  );
  assert.equal(jobDisplayName(script, t), "Artwork Encoding");
  assert.equal(jobDescription(script, t), "Housekeeping");
  assert.equal(jobScheduleDescription(script, t), "Manual only");

  const unknown = job(
    "NEW_UPSTREAM_JOB",
    "New upstream job",
    "unrecognized producer message",
    "Every 3 days",
  );
  assert.equal(jobDisplayName(unknown, t), "New upstream job");
  assert.equal(jobDescription(unknown, t), "unrecognized producer message");
  assert.equal(jobScheduleDescription(unknown, t), "Every 3 days");
});
