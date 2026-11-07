import type { ReactNode } from "react";
import { X } from "lucide-react";
import { useQuery } from "urql";

import { libraryGraphTitleDetailsQuery } from "@/lib/graphql/queries";

import { TitlePosterSlot } from "@/components/title-poster-slot";
import { TitleRatingsStrip } from "@/components/views/title-ratings-strip";
import { useTranslate } from "@/lib/context/translate-context";
import type { DiscoveryItem } from "@/lib/types/discovery";
import type { Facet, TitleRecord } from "@/lib/types/titles";
import { discoveryItemFacet } from "@/lib/utils/discovery-actions";
import {
  selectBackdropVariantUrl,
  selectPosterVariantUrl,
} from "@/lib/utils/poster-images";

type LibraryGraphSelectionCardProps = {
  label: string;
  item: DiscoveryItem | null;
  title: TitleRecord | null;
  facet: Facet;
  ghost: boolean;
  inLibrary: boolean;
  onClose: () => void;
  children: ReactNode;
};

export function LibraryGraphSelectionCard({
  label,
  item,
  title,
  facet,
  ghost,
  inLibrary,
  onClose,
  children,
}: LibraryGraphSelectionCardProps) {
  const t = useTranslate();
  // Catalog graph rows omit long descriptions and backdrop metadata. Read
  // only the picked title, leaving the full graph's initial payload lean.
  const [details] = useQuery<{
    title: Pick<
      TitleRecord,
      "id" | "overview" | "backgroundUrl" | "canonicalTags"
    > | null;
  }>({
    query: libraryGraphTitleDetailsQuery,
    variables: { id: title?.id ?? "" },
    pause:
      !title ||
      (title.overview !== undefined &&
        title.backgroundUrl !== undefined &&
        title.canonicalTags !== undefined),
  });
  const metadata =
    title && details.data?.title?.id === title.id ? details.data.title : null;

  const posterUrl = selectPosterVariantUrl(
    title?.posterUrl ?? item?.posterUrl,
    "w250",
  );
  const backdropUrl = selectBackdropVariantUrl(
    title?.backgroundUrl ?? metadata?.backgroundUrl ?? item?.backgroundUrl,
    "w1280",
  );
  const overview = title?.overview || metadata?.overview || item?.overview;
  const contentType =
    title?.facet ?? (item && discoveryItemFacet(item)) ?? facet;
  const genres = [
    ...new Set(
      (
        title?.canonicalTags ??
        metadata?.canonicalTags ??
        item?.canonicalTags ??
        []
      )
        .filter(
          (tag) => tag.category.toLowerCase() === "genre" && !tag.isSpoiler,
        )
        .map((tag) => tag.name.trim())
        .filter(Boolean),
    ),
  ].slice(0, 4);

  return (
    <section
      aria-label={label}
      data-testid="library-graph-selection"
      data-ghost={ghost ? "true" : "false"}
      className="absolute bottom-3 right-3 z-10 flex max-h-[calc(100%-1.5rem)] max-sm:max-h-[55%] w-[min(26rem,calc(100%-1.5rem))] flex-col overflow-hidden rounded-[16px] border border-[var(--scry-border2)] bg-[var(--scry-card2)] text-[13px] text-[var(--scry-ink2)] shadow-[0_16px_48px_rgba(0,0,0,0.45)]"
    >
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-x-0 top-0 h-48 overflow-hidden"
      >
        {backdropUrl ? (
          <img
            key={backdropUrl}
            src={backdropUrl}
            alt=""
            className="h-full w-full object-cover opacity-60"
            onError={(event) => {
              event.currentTarget.hidden = true;
            }}
          />
        ) : null}
        <div className="absolute inset-0 bg-gradient-to-t from-[var(--scry-card2)] via-[var(--scry-card2)]/40 to-transparent" />
      </div>
      <div className="relative min-h-0 overflow-y-auto overscroll-contain p-4 sm:p-5">
        <div className="grid grid-cols-[6rem_minmax(0,1fr)] grid-rows-[auto_1fr] gap-x-4 sm:grid-cols-[7rem_minmax(0,1fr)]">
          <TitlePosterSlot
            src={posterUrl}
            alt=""
            emptyLabel={t("title.noPoster")}
            fallbackTitle={label}
            fallbackShowText={false}
            loading="eager"
            className="row-span-2 aspect-[2/3] w-24 self-start rounded-[8px] border border-white/15 object-cover shadow-lg sm:w-28"
          />
          <button
            type="button"
            onClick={onClose}
            aria-label={t("libraryGraph.backToOverview")}
            title={t("libraryGraph.backToOverview")}
            className="mb-2 flex h-8 w-8 items-center justify-center justify-self-end rounded-full border border-white/10 bg-black/55 text-white transition hover:bg-black/80 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--scry-accent)]"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
          <div className="min-w-0 self-end space-y-2 pb-1">
            <h3 className="break-words font-[family-name:var(--font-space-grotesk)] text-[20px] font-bold leading-tight text-[var(--scry-ink)]">
              {label}
            </h3>
            <p className="text-[12px] text-[var(--scry-muted2)]">
              {t(
                contentType === "MOVIE"
                  ? "label.movie"
                  : contentType === "ANIME"
                    ? "label.anime"
                    : "label.series",
              )}
            </p>
            <span className="inline-flex items-center gap-1.5 rounded-full border border-[var(--scry-border2)] bg-[var(--scry-card2)]/80 px-2 py-1 text-[11px] font-medium">
              <span
                aria-hidden="true"
                className={`h-1.5 w-1.5 rounded-full ${inLibrary ? "bg-emerald-400" : "bg-[var(--scry-muted2)]"}`}
              />
              {t(
                inLibrary
                  ? "libraryGraph.inLibrary"
                  : "libraryGraph.notInLibrary",
              )}
            </span>
          </div>
        </div>
        <TitleRatingsStrip ratings={title?.ratings ?? item} />
        {genres.length > 0 ? (
          <div className="mt-3 flex flex-wrap gap-1.5">
            {genres.map((genre) => (
              <span
                key={genre}
                className="rounded-md border border-[var(--scry-border2)] bg-[var(--scry-inset)] px-2 py-1 text-[11px] text-[var(--scry-muted2)]"
              >
                {genre}
              </span>
            ))}
          </div>
        ) : null}
        {overview ? (
          <p className="mt-3 whitespace-pre-line text-[13px] leading-relaxed text-[var(--scry-ink2)]">
            {overview}
          </p>
        ) : null}
      </div>
      <div className="relative shrink-0 space-y-2 border-t border-[var(--scry-border2)] bg-[var(--scry-card2)] px-4 py-3 sm:px-5">
        {children}
      </div>
    </section>
  );
}
