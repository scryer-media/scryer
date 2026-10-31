import type { ScoringEntryKind } from "./release-decision-explanation.ts";

/** Who wrote the rule behind a score entry, when the source records it. */
export type ScoreLogOrigin = "user" | "system";

/** One line of a scoring log, whichever surface the log came from. */
export type ScoreLogEntry = {
  code: string;
  delta: number;
  kind?: ScoringEntryKind;
  origin?: ScoreLogOrigin;
  ruleSetName?: string;
  /** A friendlier label than the code, when the surface has one. */
  label?: string;
};

export type ScoreTone = "positive" | "negative" | "neutral";

export function formatScore(score: number | null | undefined): string {
  if (score == null) return "—";
  return score > 0 ? `+${score}` : String(score);
}

export function scoreTone(score: number | null | undefined): ScoreTone {
  if (score == null) return "neutral";
  return score < 0 ? "negative" : "positive";
}

export function scoreEntryTone(
  entry: Pick<ScoreLogEntry, "delta" | "kind">,
): ScoreTone {
  if (
    entry.kind === "mandatory_rejection" ||
    entry.kind === "final_score_rejection"
  ) {
    return "negative";
  }
  return entry.delta < 0 ? "negative" : "positive";
}

function scoringKind(value: unknown): ScoringEntryKind | undefined {
  return value === "score_contribution" ||
    value === "mandatory_rejection" ||
    value === "final_score_rejection"
    ? value
    : undefined;
}

/**
 * Read a rule source in any of the shapes the backend records: the
 * interactive search's `user:<id>` / `system:<id>` string with a separate
 * rule-set name, the acquisition log's `{ kind, name }` object, or a saved
 * evaluation's bare rule-set name.
 */
function readSource(
  source: unknown,
  ruleSetName: unknown,
): Pick<ScoreLogEntry, "origin" | "ruleSetName"> {
  const name =
    typeof ruleSetName === "string" && ruleSetName.trim()
      ? ruleSetName
      : undefined;
  if (typeof source === "string") {
    if (source.startsWith("user:"))
      return { origin: "user", ...(name ? { ruleSetName: name } : {}) };
    if (source.startsWith("system:"))
      return { origin: "system", ...(name ? { ruleSetName: name } : {}) };
    if (source === "builtin" || !source.trim())
      return name ? { ruleSetName: name } : {};
    return { ruleSetName: name ?? source };
  }
  if (source && typeof source === "object" && "kind" in source) {
    const objectName =
      "name" in source && typeof source.name === "string" && source.name.trim()
        ? source.name
        : name;
    const origin: ScoreLogOrigin | undefined =
      source.kind === "user_rule"
        ? "user"
        : source.kind === "system_rule"
          ? "system"
          : undefined;
    return {
      ...(origin ? { origin } : {}),
      ...(objectName ? { ruleSetName: objectName } : {}),
    };
  }
  return name ? { ruleSetName: name } : {};
}

/** Normalize one raw entry; returns null for anything without a code and a finite delta. */
export function toScoreLogEntry(raw: unknown): ScoreLogEntry | null {
  if (!raw || typeof raw !== "object") return null;
  const record = raw as Record<string, unknown>;
  if (typeof record.code !== "string" || !record.code.trim()) return null;
  if (typeof record.delta !== "number" || !Number.isFinite(record.delta))
    return null;
  const kind = scoringKind(record.kind);
  const origin =
    record.origin === "user" || record.origin === "system"
      ? record.origin
      : undefined;
  return {
    code: record.code,
    delta: record.delta,
    ...(kind ? { kind } : {}),
    ...readSource(record.source, record.ruleSetName ?? record.rule_set_name),
    ...(origin ? { origin } : {}),
  };
}

export function toScoreLogEntries(
  raw: readonly unknown[] | null | undefined,
): ScoreLogEntry[] {
  return (raw ?? []).flatMap((entry) => {
    const normalized = toScoreLogEntry(entry);
    return normalized ? [normalized] : [];
  });
}

export type ParsedScoreLog = { entries: ScoreLogEntry[]; total?: number };

/**
 * Parse a stored scoring log: a JSON string or value holding either the
 * entry array itself, `{ scoring_log, release_score }`, or a full decision
 * explanation with `quality_profile_decision.scoring_log`.
 */
export function parseScoreLog(value: unknown): ParsedScoreLog {
  let parsed = value;
  if (typeof parsed === "string") {
    try {
      parsed = JSON.parse(parsed);
    } catch {
      return { entries: [] };
    }
  }
  if (Array.isArray(parsed)) return { entries: toScoreLogEntries(parsed) };
  if (!parsed || typeof parsed !== "object") return { entries: [] };
  const record = parsed as Record<string, unknown>;
  const decision =
    record.quality_profile_decision &&
    typeof record.quality_profile_decision === "object"
      ? (record.quality_profile_decision as Record<string, unknown>)
      : record;
  const log = Array.isArray(decision.scoring_log) ? decision.scoring_log : [];
  const total =
    typeof decision.release_score === "number" &&
    Number.isFinite(decision.release_score)
      ? decision.release_score
      : undefined;
  return {
    entries: toScoreLogEntries(log),
    ...(total != null ? { total } : {}),
  };
}
