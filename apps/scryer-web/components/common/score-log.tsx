import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import { scoringEntryText } from "@/lib/utils/release-decision-explanation";
import {
  formatScore,
  scoreEntryTone,
  scoreTone,
  type ScoreLogEntry,
  type ScoreTone,
} from "@/lib/utils/score-log";

const toneClassNames: Record<ScoreTone, string> = {
  positive: "text-[var(--scry-success-text-soft)]",
  negative: "text-[var(--scry-danger-text-soft)]",
  neutral: "text-[var(--scry-faint)]",
};

export function scoreToneClassName(score: number | null | undefined): string {
  return toneClassNames[scoreTone(score)];
}

/** A signed release score, coloured by sign, as the interactive search shows it. */
export function ScoreValue({
  score,
  className,
  id,
}: {
  score: number | null | undefined;
  className?: string;
  id?: string;
}) {
  return (
    <span
      id={id}
      data-ui="score-value"
      className={cn(
        "font-[var(--font-code)] font-semibold tabular-nums",
        scoreToneClassName(score),
        className,
      )}
    >
      {formatScore(score)}
    </span>
  );
}

function originBadgeKey(entry: ScoreLogEntry): string | null {
  if (entry.origin === "user") return "scoring.customRule";
  if (entry.origin === "system") return "scoring.systemRule";
  return null;
}

/**
 * One scoring log: every entry with its rule badge and delta, then the
 * total. This is the interactive search's log, used wherever a score is
 * explained.
 */
export function ScoreLog({
  entries,
  total,
  title,
  id,
  className,
}: {
  entries: readonly ScoreLogEntry[];
  total?: number | null;
  /** Heading above the entries; defaults to "Scoring log", `null` hides it. */
  title?: React.ReactNode | null;
  id?: string;
  className?: string;
}) {
  const t = useTranslate();
  if (!entries.length && total == null) return null;
  const heading = title === undefined ? t("nzb.scoringLog") : title;

  return (
    <div
      id={id}
      data-ui="score-log"
      className={cn("bg-background/80 px-3 py-2", className)}
    >
      {heading ? (
        <p className="mb-1 text-xs font-semibold text-muted-foreground">
          {heading}
        </p>
      ) : null}
      <ScoreLogRows entries={entries} />
      {total != null ? (
        <div className="mt-1.5 flex justify-between border-t border-border pt-1.5 font-[var(--font-code)] text-xs font-semibold">
          <span className="text-muted-foreground">{t("nzb.total")}</span>
          <span className={scoreToneClassName(total)}>
            {formatScore(total)}
          </span>
        </div>
      ) : null}
    </div>
  );
}

/** The entry lines of a scoring log, without heading or total. */
export function ScoreLogRows({
  entries,
  className,
}: {
  entries: readonly ScoreLogEntry[];
  className?: string;
}) {
  const t = useTranslate();
  if (!entries.length) return null;
  return (
    <div className={cn("space-y-0.5", className)}>
      {entries.map((entry, index) => {
        const badgeKey = originBadgeKey(entry);
        return (
          <div
            key={`${entry.code}-${index}`}
            className="flex justify-between gap-4 font-[var(--font-code)] text-xs"
          >
            <span className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-muted-foreground">
              <span
                className="break-all"
                title={entry.label ? entry.code : undefined}
              >
                {entry.label ?? entry.code}
              </span>
              {badgeKey ? (
                <span className="rounded bg-muted px-1 py-0.5 font-sans text-[10px] leading-none text-muted-foreground">
                  {t(badgeKey)}
                </span>
              ) : null}
              {entry.ruleSetName ? (
                <span className="font-sans text-[10px] text-muted-foreground/60">
                  {entry.ruleSetName}
                </span>
              ) : null}
            </span>
            <span
              className={cn("shrink-0", toneClassNames[scoreEntryTone(entry)])}
            >
              {scoringEntryText(entry, t)}
            </span>
          </div>
        );
      })}
    </div>
  );
}

/**
 * A labelled score whose value opens its scoring log, the way a search
 * result's score does. Without entries the value is plain text.
 */
export function ScoreStat({
  label,
  score,
  entries,
  total,
  meta,
  id,
  logId,
  className,
}: {
  label: React.ReactNode;
  score: number | null | undefined;
  entries?: readonly ScoreLogEntry[];
  /** Total line for the log; defaults to the score. */
  total?: number | null;
  /** Secondary text after the score, such as when it was evaluated. */
  meta?: React.ReactNode;
  id?: string;
  logId?: string;
  className?: string;
}) {
  const t = useTranslate();
  const [expanded, setExpanded] = React.useState(false);
  const hasLog = Boolean(entries?.length);

  return (
    <div id={id} data-ui="score-stat" className={cn("text-xs", className)}>
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
        <span className="text-muted-foreground">{label}</span>
        {hasLog ? (
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation();
              setExpanded((previous) => !previous);
            }}
            id={id ? `${id}-toggle` : undefined}
            data-ui="score-log-toggle"
            aria-expanded={expanded}
            aria-label={
              expanded ? t("nzb.hideScoringLog") : t("nzb.showScoringLog")
            }
            className="underline-offset-2 hover:underline"
          >
            <ScoreValue score={score} />
          </button>
        ) : (
          <ScoreValue score={score} />
        )}
        {meta ? <span className="text-muted-foreground">{meta}</span> : null}
      </div>
      {expanded && hasLog ? (
        <ScoreLog
          id={logId}
          entries={entries ?? []}
          total={total === undefined ? score : total}
          className="mt-1 rounded-md border border-border"
        />
      ) : null}
    </div>
  );
}
