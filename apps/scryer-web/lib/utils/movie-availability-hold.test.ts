import test from "node:test";
import assert from "node:assert/strict";

import { movieAvailabilityHold } from "./movie-availability-hold.ts";
import type { MovieAvailabilityRecord } from "../types/titles.ts";

const translations: Record<string, string> = {
  "calendar.locale": "en",
  "settings.movieAvailabilityStatus.waiting": "Waiting",
  "settings.movieAvailabilityStatus.unknown": "Unknown",
  "settings.movieAvailability.estimated": "Estimated date",
  "settings.movieAvailability.reason.waiting_for_release_date":
    "Waiting for release date.",
  "settings.movieAvailability.reason.release_dates_unknown":
    "Release dates are not known yet.",
};
const translate = (key: string) => translations[key] ?? key;
const searched = { monitored: true, hasFile: false };

const waiting: MovieAvailabilityRecord = {
  status: "waiting",
  effectiveDate: "2026-12-01",
  estimated: false,
  reason: "waiting_for_release_date",
  market: "US",
  minimumAvailability: "released",
};

test("a waiting movie shows its state, effective date and reason", () => {
  assert.deepEqual(movieAvailabilityHold(waiting, translate, searched), {
    status: "waiting",
    statusLabel: "Waiting",
    dateLabel: "Dec 1, 2026",
    reasonLabel: "Waiting for release date.",
  });
});

test("an estimated date carries the Calendar's estimate label", () => {
  assert.equal(
    movieAvailabilityHold(
      { ...waiting, estimated: true },
      translate,
      searched,
    )
      ?.dateLabel,
    "Dec 1, 2026 (Estimated date)",
  );
});

test("unknown availability has no date but still says why", () => {
  assert.deepEqual(
    movieAvailabilityHold(
      {
        ...waiting,
        status: "unknown",
        effectiveDate: null,
        reason: "release_dates_unknown",
      },
      translate,
      searched,
    ),
    {
      status: "unknown",
      statusLabel: "Unknown",
      dateLabel: null,
      reasonLabel: "Release dates are not known yet.",
    },
  );
});

test("an available movie, or one not loaded yet, shows nothing", () => {
  assert.equal(
    movieAvailabilityHold(
      { ...waiting, status: "available", reason: "available" },
      translate,
      searched,
    ),
    null,
  );
  assert.equal(movieAvailabilityHold(null, translate, searched), null);
  assert.equal(movieAvailabilityHold(undefined, translate, searched), null);
});

test("no hold shows for a movie automatic search is not waiting on", () => {
  assert.equal(
    movieAvailabilityHold(waiting, translate, {
      monitored: false,
      hasFile: false,
    }),
    null,
  );
  assert.equal(
    movieAvailabilityHold(waiting, translate, {
      monitored: true,
      hasFile: true,
    }),
    null,
  );
});

test("a reason code the client does not know shows no reason", () => {
  const hold = movieAvailabilityHold(
    { ...waiting, reason: "some_future_reason" },
    translate,
    searched,
  );
  assert.equal(hold?.reasonLabel, null);
  assert.equal(hold?.statusLabel, "Waiting");
});
