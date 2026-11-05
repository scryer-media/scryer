import * as React from "react";
import { useClient } from "urql";
import { Calculator } from "lucide-react";
import { Button } from "@/components/ui/button";
import { IconButton } from "@/components/ui/icon-button";
import { TextActionButton } from "@/components/ui/text-action-button";
import { LoadingMark } from "@/components/common/loading-mark";
import { useJobRunToasts } from "@/components/root/job-run-provider";
import { JOB_RUN_FIELDS } from "@/lib/graphql/queries";
import { useTranslate } from "@/lib/context/translate-context";
import { useGlobalStatus } from "@/lib/context/global-status-context";
import { cn } from "@/lib/utils";
import { normalizeJobRun } from "@/lib/utils/job-runs";

type Selection =
  | { fileIds: string[] }
  | { episodeIds: string[] }
  | { seasonIds: string[] }
  | { titleIds: string[] };
const refresh = `mutation RefreshFileScores($selection: FileScoreSelectionInput!) {
  refreshFileScores(selection: $selection) { ${JOB_RUN_FIELDS} }
}`;
const cancel = `mutation CancelFileScoreRefresh($id: ID!) { cancelFileScoreRefresh(runId: $id) }`;

/**
 * `text` is the bulk-selection button, `icon` sits among a header's icon
 * actions, `action` sits among a file row's actions, and `menu` is a row in an
 * actions menu.
 */
export function RefreshScoresButton({
  selection,
  disabled = false,
  single = false,
  appearance = "text",
  size = "sm",
  id,
  className,
}: {
  selection: Selection;
  disabled?: boolean;
  single?: boolean;
  appearance?: "text" | "icon" | "action" | "menu";
  size?: "sm" | "default";
  id?: string;
  className?: string;
}) {
  const t = useTranslate();
  const client = useClient();
  const setStatus = useGlobalStatus();
  const { registerInteractiveJobRun, runs } = useJobRunToasts();
  const [pending, setPending] = React.useState(false);
  const [runId, setRunId] = React.useState<string | null>(null);
  const active = runs.some((run) => run.id === runId && !run.completedAt);
  const submit = async () => {
    setPending(true);
    try {
      const result = await client.mutation(refresh, { selection }).toPromise();
      if (result.error) throw result.error;
      const run = normalizeJobRun(result.data?.refreshFileScores);
      if (!run) throw new Error(t("scoring.refreshFailed"));
      setRunId(run.id);
      registerInteractiveJobRun(run);
    } catch {
      setStatus(t("scoring.refreshFailed"), { level: "ERROR" });
    } finally {
      setPending(false);
    }
  };
  const label = t(single ? "scoring.refreshOne" : "scoring.refreshSelected");
  const blocked =
    disabled || pending || active || !Object.values(selection)[0].length;
  const icon =
    pending || active ? (
      <LoadingMark className="h-4 w-4" />
    ) : (
      <Calculator className="h-4 w-4" />
    );
  return (
    <span
      className={cn(
        "items-center gap-1",
        appearance === "menu" ? "flex w-full" : "inline-flex",
      )}
      onClick={(event) => event.stopPropagation()}
      onKeyDown={(event) => event.stopPropagation()}
    >
      {appearance === "icon" ? (
        <IconButton
          id={id}
          label={label}
          className={className}
          disabled={blocked}
          onClick={() => void submit()}
        >
          {icon}
        </IconButton>
      ) : appearance === "action" ? (
        <TextActionButton
          type="button"
          size="sm"
          tone="search"
          id={id}
          className={className}
          disabled={blocked}
          onClick={() => void submit()}
          title={label}
          aria-label={label}
          leadingIcon={
            pending || active ? (
              <LoadingMark className="h-3.5 w-3.5" />
            ) : (
              <Calculator className="h-3.5 w-3.5" />
            )
          }
        >
          {label}
        </TextActionButton>
      ) : (
        <Button
          size={size}
          variant={appearance === "menu" ? "ghost" : "outline"}
          id={id}
          className={cn(
            appearance === "menu" && "flex-1 flex-row-reverse justify-start",
            className,
          )}
          disabled={blocked}
          onClick={() => void submit()}
        >
          {icon}
          {label}
        </Button>
      )}
      {active ? (
        <Button
          size="sm"
          variant="ghost"
          onClick={() =>
            void client
              .mutation(cancel, { id: runId })
              .toPromise()
              .then((result) => {
                if (result.error)
                  setStatus(t("scoring.refreshFailed"), { level: "ERROR" });
              })
          }
        >
          {t("label.cancel")}
        </Button>
      ) : null}
    </span>
  );
}

/** Reuse the existing job stream to reload visible file data after a score job finishes. */
export function useScoreRefreshUpdates(reload: () => Promise<void> | void) {
  const { runs } = useJobRunToasts();
  const terminal = runs
    .filter((run) => run.jobKey === "MEDIA_FILE_RESCORE" && run.completedAt)
    .map((run) => run.id)
    .sort()
    .join(",");
  const previous = React.useRef(new Set(terminal ? terminal.split(",") : []));
  React.useEffect(() => {
    const current = new Set(terminal ? terminal.split(",") : []);
    const newlyCompleted = [...current].some((id) => !previous.current.has(id));
    previous.current = current;
    if (newlyCompleted) {
      void reload();
    }
  }, [terminal, reload]);
}
