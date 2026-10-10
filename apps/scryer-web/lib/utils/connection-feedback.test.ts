import assert from "node:assert/strict";
import test from "node:test";

import {
  isReportedConnectionFeedbackError,
  runConnectionFeedback,
} from "./connection-feedback.ts";

test("connection feedback emits one terminal success message", async () => {
  const statuses: Array<[string, string | undefined]> = [];

  await runConnectionFeedback({
    setGlobalStatus: (status, options) => statuses.push([status, options?.level]),
    startMessage: "Testing",
    successMessage: "Connected",
    failureFallbackMessage: "Failed",
    run: async () => undefined,
  });

  // The progress note stays out of the toasts; the outcome states its level.
  assert.deepEqual(statuses, [["Testing", undefined], ["Connected", "SUCCESS"]]);
});

test("connection feedback emits one terminal failure message", async () => {
  const statuses: Array<[string, string | undefined]> = [];

  await assert.rejects(
    runConnectionFeedback({
      setGlobalStatus: (status, options) => statuses.push([status, options?.level]),
      successMessage: "Connected",
      failureFallbackMessage: "Failed",
      run: async () => {
        throw new Error("Unavailable");
      },
    }),
    isReportedConnectionFeedbackError,
  );
  assert.deepEqual(statuses, [["Unavailable", "ERROR"]]);
});
