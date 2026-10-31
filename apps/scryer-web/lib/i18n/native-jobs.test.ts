import assert from "node:assert/strict";
import test from "node:test";
import { translateNativeJobText } from "./native-jobs.ts";
import ru from "./locales/ru.ts";
import { interpolate } from "./types.ts";
const t = (
  key: string,
  values?: Record<string, string | number | boolean | null | undefined>,
) => interpolate(ru[key] ?? key, values);

test("native scheduled-job names are localized instead of exposing backend English", () => {
  assert.equal(
    translateNativeJobText("ARTWORK_ENCODING", "Artwork Encoding", t),
    "Кодирование изображений",
  );
});

test("artwork window labels localize without losing the host clock or timezone", () => {
  assert.equal(
    translateNativeJobText(
      "ARTWORK_ENCODING",
      "Daily 03:00–09:00 host local time; host now 2026-10-31 08:31 (UTC+00:00)",
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
    const output = translateNativeJobText(
      "ARTWORK_ENCODING",
      `Encode pending artwork. Run now overrides the nightly window. ${reason}.`,
      t,
    );
    assert.ok(output.endsWith(expected), output);
  }
});

test("user-defined job text and unknown metadata remain unchanged", () => {
  assert.equal(
    translateNativeJobText("CUSTOM_JOB", "Artwork Encoding", t),
    "Artwork Encoding",
  );
  assert.equal(
    translateNativeJobText(
      "NEW_UPSTREAM_JOB",
      "unrecognized producer message",
      t,
    ),
    "unrecognized producer message",
  );
});
