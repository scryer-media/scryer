export type ScriptLanguage = "SHELL" | "PYTHON" | "POWERSHELL" | "BATCH" | "GO";

export const SCRIPT_LANGUAGES: readonly ScriptLanguage[] = [
  "SHELL",
  "PYTHON",
  "POWERSHELL",
  "BATCH",
  "GO",
];

/** What starts a script: an import finishing, or its own schedule. */
export type ScriptTrigger = "POST_IMPORT" | "SCHEDULE";

export type ScriptScheduleKind = "MANUAL" | "INTERVAL" | "DAILY" | "WEEKLY" | "CRON";

export type ScheduleWeekday =
  | "MONDAY"
  | "TUESDAY"
  | "WEDNESDAY"
  | "THURSDAY"
  | "FRIDAY"
  | "SATURDAY"
  | "SUNDAY";

export const SCHEDULE_WEEKDAYS: readonly ScheduleWeekday[] = [
  "MONDAY",
  "TUESDAY",
  "WEDNESDAY",
  "THURSDAY",
  "FRIDAY",
  "SATURDAY",
  "SUNDAY",
];

/**
 * A script's schedule, the shape of both `ScriptScheduleInput` and
 * `ScriptSchedulePayload`. Only the fields the kind uses are meaningful.
 */
export type ScriptSchedule = {
  kind: ScriptScheduleKind;
  everySeconds: number | null;
  /** Local wall-clock time as `HH:MM`. */
  timeLocal: string | null;
  days: ScheduleWeekday[] | null;
  expression: string | null;
};

export type ScriptScheduleValidation = {
  valid: boolean;
  error: string | null;
  description: string | null;
  nextRuns: string[];
};

export type ScriptInterpreterSettings = {
  python: string | null;
  powershell: string | null;
  batch: string | null;
  go: string | null;
};

export type PostProcessingScript = {
  id: string;
  name: string;
  description: string;
  scriptType: string;
  scriptContent: string;
  appliedFacets: string[];
  executionMode: string;
  timeoutSecs: number;
  priority: number;
  enabled: boolean;
  debug: boolean;
  language: ScriptLanguage;
  trigger: ScriptTrigger;
  schedule: ScriptSchedule | null;
  runOnStartup: boolean;
  scheduleDescription: string | null;
  createdAt: string;
  updatedAt: string;
};

export type PostProcessingScriptRun = {
  id: string;
  scriptId: string;
  scriptName: string;
  titleId: string | null;
  titleName: string | null;
  facet: string | null;
  filePath: string | null;
  status: string;
  exitCode: number | null;
  stdoutTail: string | null;
  stderrTail: string | null;
  durationMs: number | null;
  startedAt: string;
  completedAt: string | null;
};

export type PostProcessingScriptDraft = {
  name: string;
  description: string;
  scriptType: string;
  scriptContent: string;
  appliedFacets: string[];
  executionMode: string;
  timeoutSecs: number;
  priority: number;
  enabled: boolean;
  debug: boolean;
  language: ScriptLanguage;
  trigger: ScriptTrigger;
  schedule: ScriptSchedule | null;
  runOnStartup: boolean;
};
