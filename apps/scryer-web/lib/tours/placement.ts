export type TourRect = {
  top: number;
  left: number;
  width: number;
  height: number;
};

export type TourSize = { width: number; height: number };

export type TourCardPlacement = {
  top: number;
  left: number;
  side: "center" | "above" | "below" | "left" | "right";
};

/** Space kept between the card and the viewport edge. */
export const TOUR_VIEWPORT_MARGIN = 16;
/** Space kept between the card and the spotlight, room for the arrow. */
export const TOUR_TARGET_GAP = 56;
/**
 * The least space kept between the card and the spotlight, used when no side
 * has room for the arrow: a card just clear of its target reads better than
 * one laid over it.
 */
export const TOUR_TIGHT_GAP = 12;
/** How far the spotlight extends past its target. */
export const TOUR_SPOTLIGHT_PADDING = 6;

function overlaps(a: TourRect, b: TourRect): boolean {
  return (
    a.left < b.left + b.width &&
    b.left < a.left + a.width &&
    a.top < b.top + b.height &&
    b.top < a.top + a.height
  );
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max));
}

/** The spotlight's rectangle: the target grown by the padding. */
export function spotlightRect(target: TourRect): TourRect {
  return {
    top: target.top - TOUR_SPOTLIGHT_PADDING,
    left: target.left - TOUR_SPOTLIGHT_PADDING,
    width: target.width + TOUR_SPOTLIGHT_PADDING * 2,
    height: target.height + TOUR_SPOTLIGHT_PADDING * 2,
  };
}

/**
 * Where the card goes. It sits in the middle of the viewport, as a lightbox,
 * unless that would cover the spotlight; then it moves just clear of the
 * spotlight on the side with the most room, keeping to the middle along the
 * other axis, with room for the arrow when there is any and snug against the
 * spotlight when there is not. When no side has room even then, it stays
 * centered over the page.
 */
export function placeTourCard(
  viewport: TourSize,
  card: TourSize,
  target: TourRect | null,
): TourCardPlacement {
  const margin = TOUR_VIEWPORT_MARGIN;
  const centeredLeft = clamp(
    (viewport.width - card.width) / 2,
    margin,
    viewport.width - card.width - margin,
  );
  const centeredTop = clamp(
    (viewport.height - card.height) / 2,
    margin,
    viewport.height - card.height - margin,
  );
  const center: TourCardPlacement = {
    top: centeredTop,
    left: centeredLeft,
    side: "center",
  };
  if (!target) return center;

  const spot = spotlightRect(target);
  if (
    !overlaps(
      { top: centeredTop, left: centeredLeft, ...card },
      {
        top: spot.top - TOUR_TARGET_GAP,
        left: spot.left - TOUR_TARGET_GAP,
        width: spot.width + TOUR_TARGET_GAP * 2,
        height: spot.height + TOUR_TARGET_GAP * 2,
      },
    )
  ) {
    return center;
  }

  type Side = Exclude<TourCardPlacement["side"], "center">;
  for (const gap of [TOUR_TARGET_GAP, TOUR_TIGHT_GAP]) {
    const room = {
      above: spot.top - gap - margin,
      below: viewport.height - (spot.top + spot.height) - gap - margin,
      left: spot.left - gap - margin,
      right: viewport.width - (spot.left + spot.width) - gap - margin,
    };
    const vertical: Side[] = (["below", "above"] as const)
      .filter((side) => room[side] >= card.height)
      .sort((a, b) => room[b] - room[a]);
    const horizontal: Side[] = (["right", "left"] as const)
      .filter((side) => room[side] >= card.width)
      .sort((a, b) => room[b] - room[a]);
    const side = vertical[0] ?? horizontal[0];
    if (!side) continue;

    switch (side) {
      case "below":
        return { side, left: centeredLeft, top: spot.top + spot.height + gap };
      case "above":
        return { side, left: centeredLeft, top: spot.top - gap - card.height };
      case "right":
        return { side, top: centeredTop, left: spot.left + spot.width + gap };
      case "left":
        return { side, top: centeredTop, left: spot.left - gap - card.width };
    }
  }
  return center;
}
