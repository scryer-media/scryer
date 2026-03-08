import { HardDrive } from "lucide-react";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
} from "@/components/containers/series-overview-container";
import { MediaInfoBadges } from "@/components/common/media-info-badges";
import { formatDate, formatFileSize } from "./helpers";

export function EpisodeDetailsPanel({
  episode,
  mediaFiles,
}: {
  episode: CollectionEpisode;
  mediaFiles: EpisodeMediaFile[];
}) {
  const t = useTranslate();
  return (
    <div className="space-y-3">
      {episode.overview ? (
        <div>
          <p className="mb-1 text-xs font-medium text-muted-foreground">{t("episode.overview")}</p>
          <p className="text-sm leading-relaxed text-muted-foreground">{episode.overview}</p>
        </div>
      ) : null}
      <div>
        <p className="mb-1 text-xs font-medium text-muted-foreground">{t("episode.fileOnDisk")}</p>
        {mediaFiles.length > 0 ? (
          <div className="space-y-2">
            {mediaFiles.map((file) => (
              <div key={file.id} className="space-y-1.5 rounded bg-card/60 px-3 py-2 text-sm">
                <div className="flex flex-wrap items-center gap-3">
                  <HardDrive className="h-3.5 w-3.5 shrink-0 text-muted-foreground/60" />
                  <span className="min-w-0 break-all font-mono text-xs text-muted-foreground">{file.filePath}</span>
                  {file.qualityLabel ? (
                    <span className="rounded border border-emerald-500/40 dark:border-emerald-500/30 bg-emerald-500/20 dark:bg-emerald-500/15 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">
                      {file.qualityLabel}
                    </span>
                  ) : null}
                  <span className="text-xs text-muted-foreground/60">{formatFileSize(Number(file.sizeBytes))}</span>
                  <span className="text-xs text-muted-foreground/60">{formatDate(file.createdAt)}</span>
                  {file.acquisitionScore != null ? (
                    <span className="text-xs text-muted-foreground/60" title={file.scoringLog ?? undefined}>
                      {t("mediaFile.score", { score: file.acquisitionScore })}
                    </span>
                  ) : null}
                </div>
                <MediaInfoBadges file={file} />
              </div>
            ))}
          </div>
        ) : (
          <p className="text-sm italic text-muted-foreground/60">{t("episode.noFile")}</p>
        )}
      </div>
    </div>
  );
}
