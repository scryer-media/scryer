import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { Eye, EyeOff, Loader2, Trash2, Zap } from "lucide-react";
import type { ViewId } from "@/components/root/types";
import type { TitleRecord } from "@/lib/types";
import type { ParsedQualityProfile } from "@/lib/types/quality-profiles";
import { selectPosterVariantUrl } from "@/lib/utils/poster-images";
import { useIsMobile } from "@/lib/hooks/use-mobile";
import { TitlePoster } from "@/components/title-poster";

const QP_TAG_PREFIX = "scryer:quality-profile:";

function formatProfileLabel(value: string | null | undefined): string | null {
  const trimmed = value?.trim();
  if (!trimmed) {
    return null;
  }
  if (trimmed.toLowerCase() === "4k") {
    return "4K";
  }
  if (/^\d{3,4}p$/i.test(trimmed)) {
    return trimmed.toUpperCase();
  }
  return trimmed;
}

function resolveTitleProfileName(
  title: TitleRecord,
  qualityProfiles: ParsedQualityProfile[],
  resolvedProfileName: string | null,
) {
  const tag = title.tags?.find((tg) => tg.startsWith(QP_TAG_PREFIX));
  if (tag) {
    const id = tag.slice(QP_TAG_PREFIX.length);
    const match = qualityProfiles.find((p) => p.id === id);
    if (match) return match.name;
    return formatProfileLabel(id);
  }
  return formatProfileLabel(resolvedProfileName) ?? resolvedProfileName;
}

function resolveDisplayedQualityLabel(
  title: TitleRecord,
  qualityProfiles: ParsedQualityProfile[],
  resolvedProfileName: string | null,
) {
  return resolveTitleProfileName(title, qualityProfiles, resolvedProfileName);
}

type PosterGridProps = {
  titles: TitleRecord[];
  isMovieView: boolean;
  resolvedProfileName: string | null;
  qualityProfiles: ParsedQualityProfile[];
  onOpenOverview: (targetView: ViewId, titleId: string) => void;
  onDelete: (title: TitleRecord) => void;
  onAutoQueue: (title: TitleRecord) => void;
  isDeletingById: Record<string, boolean>;
  overviewTargetView: ViewId;
};

export function PosterGrid({
  titles,
  isMovieView,
  resolvedProfileName,
  qualityProfiles,
  onOpenOverview,
  onDelete,
  onAutoQueue,
  isDeletingById,
  overviewTargetView,
}: PosterGridProps) {
  const t = useTranslate();
  const isMobile = useIsMobile();
  const [autoQueueLoadingById, setAutoQueueLoadingById] = React.useState<Record<string, boolean>>({});

  const handleAutoQueue = React.useCallback(
    (title: TitleRecord) => {
      const titleId = title.id;
      setAutoQueueLoadingById((prev) => ({ ...prev, [titleId]: true }));
      void Promise.resolve(onAutoQueue(title)).finally(() => {
        setAutoQueueLoadingById((prev) => {
          if (!prev[titleId]) return prev;
          const next = { ...prev };
          delete next[titleId];
          return next;
        });
      });
    },
    [onAutoQueue],
  );

  if (titles.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">{t("title.noManaged")}</p>
    );
  }

  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6 2xl:grid-cols-7">
      {titles.map((title) => (
        <PosterCard
          key={title.id}
          title={title}
          isMovieView={isMovieView}
          resolvedProfileName={resolvedProfileName}
          qualityProfiles={qualityProfiles}
          onOpenOverview={onOpenOverview}
          onDelete={onDelete}
          onAutoQueue={handleAutoQueue}
          deleteLoading={isDeletingById[title.id] === true}
          autoQueueLoading={autoQueueLoadingById[title.id] === true}
          overviewTargetView={overviewTargetView}
          isMobile={isMobile}
        />
      ))}
    </div>
  );
}

type PosterCardProps = {
  title: TitleRecord;
  isMovieView: boolean;
  resolvedProfileName: string | null;
  qualityProfiles: ParsedQualityProfile[];
  onOpenOverview: (targetView: ViewId, titleId: string) => void;
  onDelete: (title: TitleRecord) => void;
  onAutoQueue: (title: TitleRecord) => void;
  deleteLoading: boolean;
  autoQueueLoading: boolean;
  overviewTargetView: ViewId;
  isMobile: boolean;
};

function PosterCard({
  title,
  isMovieView,
  resolvedProfileName,
  qualityProfiles,
  onOpenOverview,
  onDelete,
  onAutoQueue,
  deleteLoading,
  autoQueueLoading,
  overviewTargetView,
  isMobile,
}: PosterCardProps) {
  const t = useTranslate();
  const posterUrl = selectPosterVariantUrl(title.posterUrl, "w250");
  const qualityLabel = resolveDisplayedQualityLabel(
    title,
    qualityProfiles,
    resolvedProfileName,
  );

  return (
    <div className="cv-auto-poster group">
      <div className="overflow-hidden rounded-lg border border-border bg-card shadow-sm">
        <div className="relative">
          <button
            type="button"
            onClick={() => onOpenOverview(overviewTargetView, title.id)}
            className="block w-full overflow-hidden bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={title.name}
          >
            <div className="relative aspect-[2/3]">
              {(posterUrl || title.posterSourceUrl) ? (
                <TitlePoster
                  src={posterUrl}
                  sourceSrc={title.posterSourceUrl}
                  alt={t("media.posterAlt", { name: title.name })}
                  className="h-full w-full object-cover"
                  loading="lazy"
                  decoding="async"
                />
              ) : (
                <div className="flex h-full w-full items-center justify-center text-sm text-muted-foreground">
                  {t("label.noArt")}
                </div>
              )}

              {!isMobile ? (
                <>
                  <div
                    aria-hidden="true"
                    className="pointer-events-none absolute inset-0 z-10 bg-black/50 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100"
                  />
                  <div className="pointer-events-none absolute inset-0 z-20 flex items-center justify-center px-3 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
                    <p className="line-clamp-3 text-center text-sm font-semibold leading-tight text-white drop-shadow-md">
                      {title.name}
                    </p>
                  </div>
                </>
              ) : null}

              <div className="absolute left-1.5 top-1.5 z-20 flex h-7 w-7 items-center justify-center rounded-full bg-black/60 backdrop-blur-sm">
                {title.monitored ? (
                  <Eye className="h-4.5 w-4.5 text-emerald-400" />
                ) : (
                  <EyeOff className="h-4.5 w-4.5 text-rose-400" />
                )}
              </div>

              {qualityLabel ? (
                <div className="absolute right-1.5 top-1.5 z-20 rounded bg-black/60 px-1.5 py-0.5 text-[10px] font-medium text-white backdrop-blur-sm">
                  {qualityLabel}
                </div>
              ) : null}

              {!isMovieView && title.contentStatus?.toLowerCase() === "ended" ? (
                <div className="absolute bottom-1.5 right-1.5 z-20 rounded bg-black/60 px-1.5 py-0.5 text-[10px] font-medium text-zinc-300 backdrop-blur-sm">
                  {t("title.ended")}
                </div>
              ) : null}
            </div>
          </button>

        </div>

        {isMobile ? (
          <div className="space-y-2 p-2">
            <button
              type="button"
              onClick={() => onOpenOverview(overviewTargetView, title.id)}
              className="block w-full text-left"
            >
              <p className="line-clamp-2 text-sm font-semibold text-foreground">{title.name}</p>
            </button>
            <div className="flex gap-2">
              {isMovieView ? (
                <Button
                  variant="secondary"
                  size="sm"
                  className="flex-1"
                  onClick={() => onAutoQueue(title)}
                  disabled={autoQueueLoading}
                >
                  {autoQueueLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Zap className="h-4 w-4" />}
                  <span>{t("label.search")}</span>
                </Button>
              ) : null}
              <Button
                variant="destructive"
                size="sm"
                className={isMovieView ? "flex-1" : "w-full"}
                onClick={() => onDelete(title)}
                disabled={deleteLoading}
              >
                {deleteLoading ? <Loader2 className="h-4 w-4 animate-spin" /> : <Trash2 className="h-4 w-4" />}
                <span>{t("label.delete")}</span>
              </Button>
            </div>
          </div>
        ) : null}
      </div>
    </div>
  );
}
