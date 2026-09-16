import assert from "node:assert/strict";
import test from "node:test";

import {
  getTitleHistoryEventLabel,
  getTitleHistoryEventMeta,
  TITLE_HISTORY_FILTERS,
  WANTED_HISTORY_FILTERS,
} from "../../components/common/title-history-event-meta.ts";
import de from "../i18n/locales/de.ts";
import en from "../i18n/locales/en.ts";
import es from "../i18n/locales/es.ts";
import fr from "../i18n/locales/fr.ts";
import it from "../i18n/locales/it.ts";
import ja from "../i18n/locales/ja.ts";
import ko from "../i18n/locales/ko.ts";
import pt_BR from "../i18n/locales/pt_BR.ts";
import zh_CN from "../i18n/locales/zh_CN.ts";

const identity = (key: string) => key;

test("download ignored is a first-class history event, not the unknown fallback", () => {
  const meta = getTitleHistoryEventMeta("download_ignored");
  assert.equal(meta.labelKey, "history.downloadIgnored");
  assert.notEqual(
    getTitleHistoryEventLabel("download_ignored", identity),
    getTitleHistoryEventLabel("some_event_that_does_not_exist", identity),
  );
  assert.ok(
    (TITLE_HISTORY_FILTERS as readonly string[]).includes("download_ignored"),
    "the filter chip has to exist, or the fixed store filter is unreachable",
  );
});

test("Activity History lists ignored downloads by default", () => {
  // The Activity History page sends this set as its event-type filter whenever
  // no chip is active, so an event missing from it is invisible there even
  // though the product recorded it. An ignored download is the only durable
  // trace of a job that left the queue without importing.
  assert.ok(
    (WANTED_HISTORY_FILTERS as readonly string[]).includes("download_ignored"),
    "download_ignored must be in the default Activity History query",
  );
  assert.ok(
    (WANTED_HISTORY_FILTERS as readonly string[]).every(event =>
      (TITLE_HISTORY_FILTERS as readonly string[]).includes(event),
    ),
    "every wanted-history filter must also be a known title-history event",
  );
});

test("every locale can name the download-ignored event", () => {
  const locales: Array<[string, Record<string, string>]> = [
    ["de", de],
    ["en", en],
    ["es", es],
    ["fr", fr],
    ["it", it],
    ["ja", ja],
    ["ko", ko],
    ["pt_BR", pt_BR],
    ["zh_CN", zh_CN],
  ];
  for (const [name, dictionary] of locales) {
    assert.equal(
      typeof dictionary["history.downloadIgnored"],
      "string",
      `missing history.downloadIgnored in ${name}`,
    );
  }
});
