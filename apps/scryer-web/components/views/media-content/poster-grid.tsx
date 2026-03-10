import * as React from "react";
import { useTranslate } from "@/lib/context/translate-context";
import { Button } from "@/components/ui/button";
import { Loader2, Trash2, Zap } from "lucide-react";
import type { ViewId } from "@/components/root/types";
import type { TitleRecord } from "@/lib/types";
import type { ParsedQualityProfile } from "@/lib/types/quality-profiles";

const QP_TAG_PREFIX = "scryer:quality-profile:";

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
}: PosterCardProps) {
  const t = useTranslate();
  const qualityLabel = isMovieView
    ? title.qualityTier
    : (() => {
        const tag = title.tags?.find((tg) => tg.startsWith(QP_TAG_PREFIX));
        if (tag) {
          const id = tag.slice(QP_TAG_PREFIX.length);
          const match = qualityProfiles.find((p) => p.id === id);
          if (match) return match.name;
        }
        return resolvedProfileName;
      })();

  return (
    <div className="cv-auto-poster group relative">
      <button
        type="button"
        onClick={() => onOpenOverview(overviewTargetView, title.id)}
        className="block w-full overflow-hidden rounded-lg border border-border bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        aria-label={title.name}
      >
        <div className="relative aspect-[2/3]">
          {title.posterUrl ? (
            <img
              src={title.posterUrl}
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

          {/* Bottom gradient with title name */}
          <div className="absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/80 via-black/40 to-transparent px-2 pb-2 pt-8">
            <p className="line-clamp-2 text-sm font-semibold leading-tight text-white">
              {title.name}
            </p>
          </div>

          {/* Monitored badge - top left */}
          {title.monitored ? (
            <div className="absolute left-1.5 top-1.5 rounded-full bg-primary/90 px-1.5 py-0.5 text-[10px] font-medium text-primary-foreground">
              {t("title.monitored")}
            </div>
          ) : null}

          {/* Quality badge - top right */}
          {qualityLabel ? (
            <div className="absolute right-1.5 top-1.5 rounded bg-black/60 px-1.5 py-0.5 text-[10px] font-medium text-white backdrop-blur-sm">
              {qualityLabel}
            </div>
          ) : null}

          {/* Hover overlay with actions */}
          <div className="absolute inset-0 flex items-center justify-center gap-2 bg-black/50 opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100">
            {isMovieView ? (
              <Button
                variant="secondary"
                size="sm"
                aria-label={t("label.search")}
                onClick={(e) => {
                  e.stopPropagation();
                  onAutoQueue(title);
                }}
                disabled={autoQueueLoading}
              >
                {autoQueueLoading ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <Zap className="h-4 w-4" />
                )}
              </Button>
            ) : null}
            <Button
              variant="destructive"
              size="sm"
              aria-label={t("label.delete")}
              onClick={(e) => {
                e.stopPropagation();
                onDelete(title);
              }}
              disabled={deleteLoading}
            >
              {deleteLoading ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Trash2 className="h-4 w-4" />
              )}
            </Button>
          </div>
        </div>
      </button>
    </div>
  );
}
