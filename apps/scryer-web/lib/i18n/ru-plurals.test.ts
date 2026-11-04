import assert from "node:assert/strict";
import test from "node:test";
import { loadLocaleDictionary, t } from "./index.ts";

test("Russian episode counts use one, few and many including teens", async () => {
  await loadLocaleDictionary("rus");
  for (const [count, noun] of [
    [0, "серий"],
    [1, "серия"],
    [2, "серии"],
    [5, "серий"],
    [11, "серий"],
    [12, "серий"],
    [14, "серий"],
    [21, "серия"],
    [22, "серии"],
    [25, "серий"],
    [101, "серия"],
    [111, "серий"],
    [1.5, "серии"],
  ] as const) {
    assert.equal(
      t("title.episodeCountOther", "ru-RU", { count }),
      `${count} ${noun}`,
    );
  }
});

test("Russian dashboard labels agree with separate numeric counters", async () => {
  await loadLocaleDictionary("rus");
  assert.equal(t("dashboard.metricMovies", "rus", { count: 1 }), "фильм");
  assert.equal(t("dashboard.metricMovies", "rus", { count: 2 }), "фильма");
  assert.equal(t("dashboard.metricMovies", "rus", { count: 11 }), "фильмов");
  assert.equal(t("dashboard.metricSeries", "rus", { count: 21 }), "сериал");
});

test("plural selection leaves English and countless translations unchanged", async () => {
  await loadLocaleDictionary("rus");
  assert.equal(t("title.episodeCountOther", "eng", { count: 2 }), "2 episodes");
  assert.equal(t("label.language", "rus", { count: 2 }), "Язык");
  assert.equal(
    t("dashboard.greetingEvening", "rus", { name: "admin" }),
    "Добрый вечер, admin",
  );
});
