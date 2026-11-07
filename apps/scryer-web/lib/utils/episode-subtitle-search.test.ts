import assert from "node:assert/strict";
import test from "node:test";
import type { JobRun } from "../types/jobs.ts";
import {
  completedSubtitleBatchIds,
  episodeSelectionActions,
  isEpisodeSubtitleBatch,
  subtitleBatchErrorMessage,
  subtitleBatchProgressText,
  submitEpisodeSubtitleBatch,
} from "./episode-subtitle-search.ts";

const run = {
  id: "batch-1",
  jobKey: "SUBTITLE_SEARCH",
  status: "RUNNING",
  completedAt: null,
  progressJson: { kind: "episode_subtitle_search", completed: 1, total: 3 },
  summaryJson: null,
} as JobRun;
const t = (key: string, values?: Record<string, unknown>) =>
  `${key}${values ? JSON.stringify(values) : ""}`;

test("subtitle-only permission enables selection without delete", () => {
  assert.deepEqual(episodeSelectionActions(false, true), {
    enabled: true,
    canDelete: false,
    canSearchSubtitles: true,
  });
  assert.equal(episodeSelectionActions(false, false).enabled, false);
});

test("selection clears only after accepted job registration, without waiting for completion", async () => {
  const calls: string[] = [];
  await submitEpisodeSubtitleBatch(
    async () => run,
    (accepted) => {
      assert.equal(accepted.status, "RUNNING");
      calls.push("registered");
    },
    () => calls.push("cleared"),
  );
  assert.deepEqual(calls, ["registered", "cleared"]);
});

test("rejected submission preserves selection and does not register a job", async () => {
  await assert.rejects(
    submitEpisodeSubtitleBatch(
      async () => {
        throw new Error("rejected");
      },
      () => assert.fail("must not register"),
      () => assert.fail("must not clear"),
    ),
  );
});

test("progress distinguishes selected batches from periodic subtitle searches", () => {
  assert.equal(isEpisodeSubtitleBatch(run), true);
  assert.equal(isEpisodeSubtitleBatch({ ...run, progressJson: {} }), false);
  assert.equal(
    subtitleBatchProgressText(run, t),
    'subtitle.batchProgress{"completed":1,"total":3}',
  );
  const completed = {
    ...run,
    status: "WARNING",
    completedAt: "now",
    progressJson: { status: "warning" },
    summaryJson: {
      kind: "episode_subtitle_search",
      downloaded: 2,
      warnings: 1,
    },
  } as JobRun;
  assert.equal(isEpisodeSubtitleBatch(completed), true);
  assert.equal(
    subtitleBatchProgressText(completed, t),
    'subtitle.batchCompleted{"downloaded":2,"warnings":1}',
  );
});

test("refusals use stable reason codes regardless of server wording", () => {
  const error = {
    graphQLErrors: [
      {
        message: "any sentence",
        extensions: {
          code: "VALIDATION_ERROR",
          reason: "SUBTITLES_BATCH_ALREADY_RUNNING",
        },
      },
    ],
  };
  assert.equal(
    subtitleBatchErrorMessage(error, t),
    "subtitle.batchAlreadyRunning",
  );
});

test("refresh tracks only completed subtitle batches, including partial failures", () => {
  const done = { ...run, completedAt: "now", status: "WARNING" } as JobRun;
  assert.deepEqual([...completedSubtitleBatchIds([run])], []);
  assert.deepEqual(
    [
      ...completedSubtitleBatchIds([
        done,
        { ...done, id: "periodic", progressJson: {} },
      ]),
    ],
    ["batch-1"],
  );
});
