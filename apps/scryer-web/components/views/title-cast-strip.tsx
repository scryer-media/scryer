import { Users } from "lucide-react";

import { HorizontalRail } from "@/components/common/horizontal-scroll-fade";
import {
  TitleWorkspaceSectionCard,
  TitleWorkspaceSectionHeader,
} from "@/components/views/media-content/title-workspace-primitives";
import { useTranslate } from "@/lib/context/translate-context";
import type { TitleCreditRecord } from "@/lib/types/titles";
import {
  isTitleCastPlaceholder,
  titleCastCreditCharacter,
  titleCastCreditEpisodeCount,
  titleCastCreditKey,
  titleCastCredits,
  type TitleCastDisplayCredit,
} from "@/lib/utils/title-cast";

/**
 * `"panel"` matches the series-overview page chrome, `"workspace"` the title
 * context panel's section cards. The rail itself is identical either way.
 */
export type TitleCastStripVariant = "panel" | "workspace";

type Props = {
  credits?: TitleCastDisplayCredit[] | null;
  variant?: TitleCastStripVariant;
  /** i18n heading key; the dub rail passes `title.dubCast`. */
  titleKey?: string;
  /** Rendered beside the heading; the dub rail passes its language picker. */
  headerAccessory?: React.ReactNode;
  /**
   * Keep person-less entries. The aligned dub rail relies on this: its
   * placeholder slots hold a column open so the dub portraits stay under the
   * matching original ones.
   */
  keepPlaceholders?: boolean;
};

/**
 * One cast rail for a title, rendered from the cached credits that ride the
 * overview snapshot. Callers pass a pre-split list (original cast vs dub cast
 * from lib/utils/title-cast); the strip renders that order verbatim and
 * disappears when the list is empty.
 *
 * Cards are deliberately non-interactive: there are no person pages yet.
 */
export function TitleCastStrip({
  credits,
  variant = "panel",
  titleKey = "title.topBilledCast",
  headerAccessory,
  keepPlaceholders = false,
}: Props) {
  const t = useTranslate();
  const cast = keepPlaceholders ? (credits ?? []) : titleCastCredits(credits);
  const cardGapPx = variant === "workspace" ? 11 : 12;

  if (cast.length === 0) {
    return null;
  }

  const heading = t(titleKey);
  const cards = (
    <HorizontalRail
      className={
        variant === "workspace"
          ? "flex gap-[11px] overflow-x-auto pb-1"
          : "flex gap-3 overflow-x-auto pb-1"
      }
    >
      {cast.map((credit, index) => (
        <TitleCastCard
          key={titleCastCreditKey(credit, index)}
          credit={credit}
          episodeLabel={castCreditEpisodeLabel(credit, t)}
          gapPx={cardGapPx}
        />
      ))}
    </HorizontalRail>
  );

  if (variant === "workspace") {
    return (
      <TitleWorkspaceSectionCard className="rounded-[14px] bg-[var(--scry-surf)]">
        <div className="flex items-center justify-between gap-3">
          <TitleWorkspaceSectionHeader icon={Users} title={heading} />
          {headerAccessory}
        </div>
        {cards}
      </TitleWorkspaceSectionCard>
    );
  }

  return (
    <section className="space-y-3 rounded-lg border border-border/70 bg-card/60 p-4">
      <div className="flex items-center gap-2">
        <span className="flex size-8 items-center justify-center rounded-lg bg-primary/15 text-primary">
          <Users className="h-4 w-4" />
        </span>
        <h2 className="text-sm font-semibold uppercase tracking-[0.08em] text-muted-foreground">
          {heading}
        </h2>
        {headerAccessory ? (
          <div className="ml-auto">{headerAccessory}</div>
        ) : null}
      </div>
      {cards}
    </section>
  );
}

function castCreditEpisodeLabel(
  credit: TitleCreditRecord,
  t: ReturnType<typeof useTranslate>,
): string | null {
  const count = titleCastCreditEpisodeCount(credit);
  if (count === null) {
    return null;
  }
  return t(count === 1 ? "title.episodeCountOne" : "title.episodeCountOther", {
    count,
  });
}

function TitleCastCard({
  credit,
  episodeLabel,
  gapPx,
}: {
  credit: TitleCastDisplayCredit;
  episodeLabel: string | null;
  gapPx: number;
}) {
  const character = titleCastCreditCharacter(credit);
  // The server hands back the `w185` portrait variant, which is already the
  // card size; no re-varianting needed.
  const portraitUrl = credit.personImageUrl ?? null;
  // A placeholder slot holds this character's column open on the dub rail; it
  // is deliberately dimmed rather than hidden, since hiding it would shift
  // every later portrait out from under its original-cast counterpart.
  const placeholder = isTitleCastPlaceholder(credit);
  const slotSpan = Math.max(1, Math.trunc(credit.slotSpan ?? 1));

  return (
    <div
      className={placeholder ? "flex shrink-0 justify-center opacity-45" : "flex shrink-0 justify-center"}
      style={
        slotSpan > 1
          ? { width: `calc(${slotSpan * 6}rem + ${(slotSpan - 1) * gapPx}px)` }
          : undefined
      }
    >
      <div className="w-24">
        <div className="aspect-[2/3] w-full overflow-hidden rounded-[10px] border border-border/60 bg-muted">
          {portraitUrl ? (
            <img
              src={portraitUrl}
              alt=""
              loading="lazy"
              decoding="async"
              className="h-full w-full object-cover"
            />
          ) : (
            <div className="flex h-full w-full items-center justify-center text-muted-foreground">
              <Users className="h-5 w-5" aria-hidden="true" />
            </div>
          )}
        </div>
        <p
          className="mt-1.5 truncate text-[12px] font-semibold text-foreground"
          title={placeholder ? undefined : credit.personName}
        >
          {placeholder ? "—" : credit.personName}
        </p>
        {character ? (
          <p className="truncate text-[11px] text-muted-foreground" title={character}>
            {character}
          </p>
        ) : null}
        {episodeLabel ? (
          <p className="truncate text-[11px] text-muted-foreground">
            {episodeLabel}
          </p>
        ) : null}
      </div>
    </div>
  );
}
