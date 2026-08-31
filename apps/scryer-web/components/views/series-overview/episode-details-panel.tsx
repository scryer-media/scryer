import * as React from "react";

import { ArtworkFallback } from "@/components/artwork-fallback";
import { useTranslate } from "@/lib/context/translate-context";
import type {
  CollectionEpisode,
  EpisodeMediaFile,
} from "@/components/containers/series-overview-container";
import { MediaFilesOnDiskPanel } from "@/components/common/media-files-on-disk-panel";
import { TitleFilesOnDiskRail } from "@/components/common/title-files-on-disk-rail";
import { WatchInMediaServerMenu } from "@/components/common/watch-in-media-server-menu";
import type { ExternalSubtitleRecord } from "@/lib/types/subtitles";
import { selectorId } from "@/lib/utils/dom-ids";
import { selectMediaImageVariantUrl } from "@/lib/utils/poster-images";

export function EpisodeDetailsPanel({
  episode,
  facet,
  mediaFiles,
  subtitleDownloads = [],
  onRefreshSubtitles,
  onDeleteFile,
  onMakePrimaryFile,
  primaryMovieFileUpdatingId = null,
}: {
  episode: CollectionEpisode;
  facet: string;
  mediaFiles: EpisodeMediaFile[];
  subtitleDownloads?: ExternalSubtitleRecord[];
  onRefreshSubtitles?: () => Promise<void> | void;
  onDeleteFile?: (fileId: string) => void;
  onMakePrimaryFile?: (fileId: string) => Promise<void> | void;
  primaryMovieFileUpdatingId?: string | null;
}) {
  const t = useTranslate();
  const episodeImageUrl = selectMediaImageVariantUrl(
    episode.imageUrl,
    "w300",
  );
  const episodeImageAlt = episode.title ?? episode.episodeLabel ?? "";
  const [imageFailed, setImageFailed] = React.useState(false);
  React.useEffect(() => {
    setImageFailed(false);
  }, [episodeImageUrl]);
  const fallbackTone = facet.trim().toUpperCase() === "ANIME" ? "ANIME" : "SERIES";
  return (
    <div id={selectorId("series-overview-episode-details", episode.id)} className="space-y-3">
      <div className="flex items-start gap-4">
        <div className="flex w-40 shrink-0 flex-col items-start gap-2 sm:w-48">
          {episodeImageUrl && !imageFailed ? (
            <img
              src={episodeImageUrl}
              alt={episodeImageAlt}
              loading="lazy"
              decoding="async"
              className="w-full rounded border border-border/70 bg-muted [image-rendering:smooth]"
              onError={() => setImageFailed(true)}
            />
          ) : (
            <ArtworkFallback
              className="aspect-video w-full rounded border border-border/70"
              ariaLabel={episodeImageAlt}
              emptyLabel={t("label.noArt")}
              title={episode.title ?? episode.episodeLabel ?? episode.id}
              tone={fallbackTone}
              showText={false}
            />
          )}
          <WatchInMediaServerMenu
            links={episode.playbackLinks}
            showLabel
            className="w-full justify-start"
          />
        </div>
        {episode.overview ? (
          <div className="min-w-0 flex-1">
            <p className="mb-1 text-xs font-medium text-muted-foreground">{t("episode.overview")}</p>
            <p className="text-sm leading-relaxed text-muted-foreground">{episode.overview}</p>
          </div>
        ) : null}
        </div>
      <TitleFilesOnDiskRail>
        <MediaFilesOnDiskPanel<EpisodeMediaFile>
          emptyMessage={t("title.noFilesTracked")}
          emptyHint={t("title.noFilesTrackedHint")}
          mediaFiles={mediaFiles}
          subtitleDownloads={subtitleDownloads}
          onRefreshSubtitles={onRefreshSubtitles}
          onDeleteFile={onDeleteFile}
          onMakePrimaryFile={onMakePrimaryFile}
          primaryFileUpdatingId={primaryMovieFileUpdatingId}
          showPrimaryRoleBadge
          fileRowIdPrefix="series-overview-episode-media-file"
          filePathIdPrefix="series-overview-episode-media-file-path"
          roleIdPrefix="series-overview-episode-media-file-role"
          subtitleSearchIdPrefix="series-overview-episode-search-subtitles"
          deleteFileIdPrefix="series-overview-episode-delete-file"
          makePrimaryFileIdPrefix="series-overview-episode-make-primary-file"
        />
      </TitleFilesOnDiskRail>
    </div>
  );
}
