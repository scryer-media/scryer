import { useTranslate } from "@/lib/context/translate-context";
import type { ManualDiscInventory, ManualDiscSelection, ManualImportVideoFacts } from "@/lib/utils/manual-import-video-facts";

type Props = {
  disc: ManualDiscInventory | null;
  report: ManualImportVideoFacts["report"];
  isMovie: boolean;
  episodes: { id: string; label: string }[];
  value: ManualDiscSelection | null;
  onChange: (value: ManualDiscSelection | null) => void;
};

export function ManualDiscSelectionControl({ disc, report, isMovie, episodes, value, onChange }: Props) {
  const t = useTranslate();
  const titles = disc?.titles ?? [];
  return (
    <div className="min-w-64 space-y-2 text-xs">
      <p>{t("mediaInfo.discIntactImport")}</p>
      {report && report.status.toLowerCase() !== "complete" && (
        <p className="text-amber-600">{t("mediaInfo.discReviewRequired")} ({report.status})</p>
      )}
      {report?.warnings.map((warning, index) => <p key={`${warning.code}-${index}`} className="text-muted-foreground">{warning.message}</p>)}
      {isMovie ? (
        <label className="flex flex-col gap-1">
          {t("mediaInfo.discImportMovie")}
          <select
            className="rounded border bg-background p-2"
            value={value ? value.titleId ?? "__automatic__" : ""}
            onChange={(event) => onChange(event.target.value ? {
              titleId: event.target.value === "__automatic__" ? null : event.target.value,
              episodeMappings: [],
            } : null)}
          >
            <option value="">{t("mediaInfo.discSkip")}</option>
            {titles.some((title) => title.report.status.toLowerCase() === "complete") && (
              <option value="__automatic__">{t("mediaInfo.discAutomaticChoice")} ({disc?.selectedTitleId ?? "?"})</option>
            )}
            {titles.map((title) => <option key={title.id} value={title.id}>
              {title.id} · {title.durationSeconds?.toFixed(1) ?? "?"} s · {title.report.status}
            </option>)}
          </select>
        </label>
      ) : titles.map((title) => {
        const episodeId = value?.episodeMappings.find((mapping) => mapping.discTitleId === title.id)?.episodeId ?? "";
        return (
          <label key={title.id} className="flex flex-col gap-1">
            <span>{title.id} · {title.durationSeconds?.toFixed(1) ?? "?"} s · {title.report.status}</span>
            <select
              className="rounded border bg-background p-2"
              value={episodeId}
              onChange={(event) => {
                const episodeMappings = (value?.episodeMappings ?? []).filter((mapping) => mapping.discTitleId !== title.id);
                if (event.target.value) episodeMappings.push({ discTitleId: title.id, episodeId: event.target.value });
                onChange(episodeMappings.length ? { titleId: value?.titleId ?? null, episodeMappings } : null);
              }}
            >
              <option value="">{t("mediaInfo.discSkip")}</option>
              {episodes.map((episode) => <option key={episode.id} value={episode.id} disabled={
                episode.id !== episodeId && value?.episodeMappings.some((mapping) => mapping.episodeId === episode.id)
              }>{episode.label}</option>)}
            </select>
          </label>
        );
      })}
      {!titles.length && <p>{t("mediaInfo.discNoTitles")}</p>}
    </div>
  );
}
