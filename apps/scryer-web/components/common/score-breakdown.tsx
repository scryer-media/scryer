import { ScoreStat } from "@/components/common/score-log";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiDateTimeFormat } from "@/lib/context/ui-settings-context";
import { formatUiDateTime } from "@/lib/utils/date-format";
import { parseScoreLog, toScoreLogEntries } from "@/lib/utils/score-log";
import type { FileScoreEvaluation } from "@/lib/types/file-score";

export function FileScoreSummary({
  evaluation,
  acquisitionScore,
  scoringLog,
  idPrefix,
}: {
  evaluation?: FileScoreEvaluation | null;
  acquisitionScore?: number | null;
  /** The log saved when the file was grabbed and imported. */
  scoringLog?: string | null;
  idPrefix?: string;
}) {
  const t = useTranslate();
  const dateTimeFormat = useUiDateTimeFormat();
  // A file shows a current score only once it has been rescored; until then
  // the score it was grabbed with stands on its own.
  if (!evaluation && acquisitionScore == null) return null;
  const acquisitionLog = parseScoreLog(scoringLog);
  const flags = evaluation
    ? [
        evaluation.needsRefresh ? t("scoring.needsRefresh") : null,
        evaluation.timeSensitive ? t("scoring.timeSensitive") : null,
      ].filter((flag): flag is string => flag !== null)
    : [];

  return (
    <div className="space-y-1">
      {acquisitionScore != null ? (
        <ScoreStat
          id={idPrefix ? `${idPrefix}-acquisition-score` : undefined}
          logId={idPrefix ? `${idPrefix}-acquisition-scoring-log` : undefined}
          label={t("mediaInfo.acquisitionScore")}
          score={acquisitionScore}
          entries={acquisitionLog.entries}
          total={acquisitionLog.total ?? acquisitionScore}
        />
      ) : null}
      {evaluation ? (
        <ScoreStat
          id={idPrefix ? `${idPrefix}-current-score` : undefined}
          logId={idPrefix ? `${idPrefix}-current-scoring-log` : undefined}
          label={t("scoring.current")}
          score={evaluation.score}
          entries={toScoreLogEntries(evaluation.entries)}
          meta={
            <>
              <time dateTime={evaluation.evaluatedAt}>
                {formatUiDateTime(evaluation.evaluatedAt, dateTimeFormat)}
              </time>
              {flags.length ? ` · ${flags.join(" · ")}` : ""}
            </>
          }
        />
      ) : null}
      {evaluation?.rejectionCodes.length ? (
        <div className="text-xs text-[var(--scry-danger-text-soft)]">
          {t("scoring.rejected")}:{" "}
          <span className="font-[var(--font-code)]">
            {evaluation.rejectionCodes.join(", ")}
          </span>
        </div>
      ) : null}
    </div>
  );
}
