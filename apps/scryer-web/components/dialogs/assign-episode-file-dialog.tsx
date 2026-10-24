import * as React from "react";

import { EpisodeTargetSelect, type EpisodeTargetPickerLabels } from "@/components/common/episode-target-picker";
import { LoadingMark } from "@/components/common/loading-mark";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useTranslate } from "@/lib/context/translate-context";
import {
  episodeSelectionTargetValue,
  episodeTargetIds,
  episodeTargetLabel,
  episodeTargetValue,
  groupEpisodesBySeason,
  sameEpisodeSelection,
  seasonGroupNumber,
  type EpisodeTargetEpisode,
} from "@/lib/utils/episode-target";

/** What the caller's assignment attempt came back with. */
export type AssignEpisodeFileOutcome =
  | { kind: "assigned" }
  /** A target episode already has a primary file; the caller may replace it. */
  | { kind: "conflict" }
  | { kind: "error"; message: string };

type Props = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Prefix of every element id in the dialog. */
  idPrefix: string;
  fileName: string | null;
  description?: string;
  originalReleaseName?: string | null;
  /** The title's episodes; null while they load. */
  episodes: readonly EpisodeTargetEpisode[] | null;
  loadError: string | null;
  /** The episodes the file is linked to now; the picker starts on them. */
  initialEpisodeIds: readonly string[];
  /** Whether choosing the current episodes again is a no-op to refuse. */
  requireChange: boolean;
  /** Whether a primary-file conflict can be resolved by replacing it. */
  replaceSupported: boolean;
  onConfirm: (episodeIds: string[], replace: boolean) => Promise<AssignEpisodeFileOutcome>;
};

export function AssignEpisodeFileDialog({
  open,
  onOpenChange,
  idPrefix,
  fileName,
  description,
  originalReleaseName,
  episodes,
  loadError,
  initialEpisodeIds,
  requireChange,
  replaceSupported,
  onConfirm,
}: Props) {
  const t = useTranslate();
  const [value, setValue] = React.useState(() => episodeSelectionTargetValue(initialEpisodeIds));
  const [applying, setApplying] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [conflict, setConflict] = React.useState(false);
  const initialKey = initialEpisodeIds.join(",");

  React.useEffect(() => {
    if (!open) return;
    setValue(episodeSelectionTargetValue(initialKey ? initialKey.split(",") : []));
    setError(null);
    setConflict(false);
    setApplying(false);
  }, [initialKey, open]);

  const groupedEpisodes = React.useMemo(
    () => groupEpisodesBySeason(episodes ?? []),
    [episodes],
  );
  const targetLabels = React.useMemo(() => {
    const labels = new Map<string, string>();
    (episodes ?? []).forEach((episode) => {
      labels.set(episodeTargetValue([episode.id]), episodeTargetLabel(episode, t));
    });
    return labels;
  }, [episodes, t]);
  const pickerLabels = React.useMemo<EpisodeTargetPickerLabels>(
    () => ({
      seriesMovies: "",
      seasonGroup: (groupKey) =>
        t("episodeAssign.seasonGroup", { season: seasonGroupNumber(groupKey) }),
      filterPlaceholder: t("episodeAssign.filterPlaceholder"),
      filterLabel: t("episodeAssign.filterLabel"),
      listLabel: t("episodeAssign.listLabel"),
      noMatches: t("episodeAssign.noMatches"),
      selectPlaceholder: t("episodeAssign.selectPlaceholder"),
    }),
    [t],
  );

  const selectedIds = React.useMemo(() => episodeTargetIds(value), [value]);
  const unchanged = requireChange && sameEpisodeSelection(selectedIds, initialEpisodeIds);
  const canApply = !applying && episodes !== null && selectedIds.length > 0 && !unchanged;

  const handleChange = React.useCallback((next: string) => {
    setValue(next);
    setConflict(false);
    setError(null);
  }, []);

  const apply = React.useCallback(
    async (replace: boolean) => {
      if (selectedIds.length === 0) return;
      setApplying(true);
      setError(null);
      try {
        const outcome = await onConfirm(selectedIds, replace);
        if (outcome.kind === "assigned") {
          onOpenChange(false);
        } else if (outcome.kind === "conflict" && replaceSupported) {
          setConflict(true);
        } else if (outcome.kind === "error") {
          setError(outcome.message);
        }
      } finally {
        setApplying(false);
      }
    },
    [onConfirm, onOpenChange, replaceSupported, selectedIds],
  );

  return (
    <Dialog open={open} onOpenChange={(next) => (applying ? undefined : onOpenChange(next))}>
      <DialogContent id={`${idPrefix}-dialog`} className="max-h-[90dvh] overflow-y-auto sm:max-w-lg" aria-describedby={description ? `${idPrefix}-description` : undefined}>
        <DialogHeader>
          <DialogTitle>{t("episodeAssign.dialogTitle")}</DialogTitle>
          {description ? <DialogDescription id={`${idPrefix}-description`}>{description}</DialogDescription> : null}
        </DialogHeader>
        <div className="space-y-4">
          {fileName ? (
            <div className="break-all rounded-md border border-border bg-muted/30 px-3 py-2 font-[var(--font-code)] text-xs text-muted-foreground">
              {fileName}
            </div>
          ) : null}
          {originalReleaseName?.trim() ? (
            <div className="min-w-0 space-y-1.5">
              <div className="text-xs font-medium text-muted-foreground">
                {t("episodeAssign.originalReleaseName")}
              </div>
              <div className="break-all rounded-md border border-border bg-muted/30 px-3 py-2 font-[var(--font-code)] text-xs text-foreground">
                {originalReleaseName.trim()}
              </div>
            </div>
          ) : null}
          {loadError ? (
            <div
              id={`${idPrefix}-load-error`}
              className="rounded-md border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]"
            >
              {loadError}
            </div>
          ) : episodes === null ? (
            <div className="flex items-center gap-2 rounded-md border border-border px-3 py-6 text-sm text-muted-foreground">
              <LoadingMark className="h-4 w-4" />
              {t("episodeAssign.loadingEpisodes")}
            </div>
          ) : (
            <div className="space-y-1.5">
              <div className="text-xs font-medium text-muted-foreground">
                {t("episodeAssign.targetLabel")}
              </div>
              <EpisodeTargetSelect
                triggerId={`${idPrefix}-target`}
                idPrefix={idPrefix}
                value={value}
                groupedEpisodes={groupedEpisodes}
                seriesMovies={[]}
                targetLabels={targetLabels}
                labels={pickerLabels}
                onChange={handleChange}
              />
              {unchanged && selectedIds.length > 0 ? (
                <p className="text-xs text-muted-foreground">{t("episodeAssign.unchanged")}</p>
              ) : null}
            </div>
          )}
          {conflict ? (
            <div
              id={`${idPrefix}-replace-confirm`}
              className="space-y-1 rounded-md border border-[var(--scry-warning-border)] bg-[var(--scry-warning-bg)] px-3 py-2 text-sm text-[var(--scry-warning-text)]"
            >
              <p className="font-medium">{t("episodeAssign.replaceTitle")}</p>
              <p>{t("episodeAssign.replaceDescription")}</p>
            </div>
          ) : null}
          {error ? (
            <div
              id={`${idPrefix}-error`}
              className="rounded-md border border-[var(--scry-danger-border)] bg-[var(--scry-danger-bg)] px-3 py-2 text-sm text-[var(--scry-danger-text)]"
            >
              {error}
            </div>
          ) : null}
        </div>
        <DialogFooter>
          <Button
            id={`${idPrefix}-cancel`}
            type="button"
            variant="ghost"
            onClick={() => onOpenChange(false)}
            disabled={applying}
          >
            {t("label.cancel")}
          </Button>
          <Button
            id={conflict ? `${idPrefix}-replace` : `${idPrefix}-apply`}
            type="button"
            onClick={() => void apply(conflict)}
            disabled={!canApply}
          >
            {applying ? (
              <>
                <LoadingMark className="mr-2 h-4 w-4" />
                {t("episodeAssign.applying")}
              </>
            ) : conflict ? (
              t("episodeAssign.replaceApply")
            ) : (
              t("episodeAssign.apply")
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
