import assert from "node:assert/strict";
import test from "node:test";
import {
  formatMoveEta,
  locationOperationIdFromJobRun,
  moveEtaSeconds,
  moveToastVisualState,
} from "./location-move-toasts.ts";

test("only a location job run with a written operation id names an operation", () => {
  assert.equal(
    locationOperationIdFromJobRun({
      jobKey: "LOCATION_OPERATION",
      progressJson: { phase: "moving", operationId: " op-1 " },
    }),
    "op-1",
  );
  assert.equal(
    locationOperationIdFromJobRun({
      jobKey: "RSS_SYNC",
      progressJson: { operationId: "op-1" },
    }),
    null,
  );
  assert.equal(
    locationOperationIdFromJobRun({
      jobKey: "LOCATION_OPERATION",
      progressJson: { operationId: "" },
    }),
    null,
  );
  assert.equal(
    locationOperationIdFromJobRun({
      jobKey: "LOCATION_OPERATION",
      progressJson: '{"operationId":"op-1"}',
    }),
    null,
  );
  assert.equal(
    locationOperationIdFromJobRun({ jobKey: "LOCATION_OPERATION", progressJson: null }),
    null,
  );
});

test("every unsettled state reads as moving; the four terminal states have their own look", () => {
  assert.equal(moveToastVisualState(null), "moving");
  assert.equal(moveToastVisualState("QUEUED"), "moving");
  assert.equal(moveToastVisualState("VERIFYING"), "moving");
  assert.equal(moveToastVisualState("CLEANING_UP"), "moving");
  assert.equal(moveToastVisualState("COMPLETED"), "success");
  assert.equal(moveToastVisualState("COMPLETED_WITH_WARNINGS"), "issues");
  assert.equal(moveToastVisualState("FAILED"), "failed");
  assert.equal(moveToastVisualState("CANCELED"), "canceled");
});

test("an ETA is shown only when the server has a positive estimate", () => {
  assert.equal(moveEtaSeconds(null), null);
  assert.equal(moveEtaSeconds(undefined), null);
  assert.equal(moveEtaSeconds("0"), null);
  assert.equal(moveEtaSeconds("not a number"), null);
  assert.equal(moveEtaSeconds("125"), 125);
  assert.equal(moveEtaSeconds(40), 40);
});

test("the ETA clock grows an hours field only once it needs one", () => {
  assert.equal(formatMoveEta(0), "00:01");
  assert.equal(formatMoveEta(0.4), "00:01");
  assert.equal(formatMoveEta(65), "01:05");
  assert.equal(formatMoveEta(3599), "59:59");
  assert.equal(formatMoveEta(3600), "1:00:00");
  assert.equal(formatMoveEta(3725.4), "1:02:05");
  assert.equal(formatMoveEta(36_000), "10:00:00");
});
