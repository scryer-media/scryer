import * as React from "react";
import type { TourRect } from "@/lib/tours/placement";
import type { TourTarget } from "@/lib/tours/types";
import { selectorId } from "@/lib/utils/dom-ids";

/** How long a step waits for its target to render before showing without it. */
export const TARGET_WAIT_MS = 1200;
/**
 * The wait for a target on a page the step has just opened: the page's code
 * and data load first, and giving up early would drop the spotlight only to
 * bring it back a moment later.
 */
export const ROUTE_TARGET_WAIT_MS = 4000;

export type TourTargetState = {
  /** The target's viewport rectangle, or null when the step shows without one. */
  rect: TourRect | null;
  /** The search is over: the target was found, or the step goes on without it. */
  settled: boolean;
  /**
   * The id of the element `rect` belongs to. While a new target is awaited
   * the rect is still the old element's, and this says so.
   */
  elementId: string | null;
};

function visibleRect(element: Element): TourRect | null {
  const box = element.getBoundingClientRect();
  if (box.width <= 0 || box.height <= 0) return null;
  return { top: box.top, left: box.left, width: box.width, height: box.height };
}

/** Rendered and not hidden away, e.g. a closed panel slid off with `visibility: hidden`. */
function isShown(element: HTMLElement): boolean {
  return typeof element.checkVisibility === "function"
    ? element.checkVisibility({
        visibilityProperty: true,
        opacityProperty: true,
      })
    : true;
}

/**
 * The step's target when it is showing, otherwise its fallback when that is
 * showing (e.g. the button that opens a closed panel), otherwise the target.
 */
function pickElement(
  id: string,
  fallbackId: string | null,
): HTMLElement | null {
  const primary = document.getElementById(id);
  if (primary && isShown(primary)) return primary;
  const fallback = fallbackId ? document.getElementById(fallbackId) : null;
  if (fallback && isShown(fallback)) return fallback;
  return primary;
}

function sameRect(a: TourRect | null, b: TourRect | null): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  return (
    Math.abs(a.top - b.top) < 0.5 &&
    Math.abs(a.left - b.left) < 0.5 &&
    Math.abs(a.width - b.width) < 0.5 &&
    Math.abs(a.height - b.height) < 0.5
  );
}

/**
 * Finds a step's target by its durable id and follows it: the spotlight moves
 * with scrolling, resizing and layout shifts. A target that is not rendered
 * yet (a route still loading) is waited for up to `waitMs`; one that never appears,
 * or is hidden, leaves the step to show centered without a spotlight. A
 * `fallback` is spotlighted while the target is hidden, and the spotlight
 * moves to the target once it shows.
 *
 * Mount once per step (key the caller by step) so each search starts fresh.
 * When the target changes within a step, the spotlight stays on the old one
 * until the new one appears, so it glides across instead of blinking out.
 */
export function useTourTarget(
  target: TourTarget | undefined,
  reduceMotion: boolean,
  fallback?: TourTarget,
  waitMs: number = TARGET_WAIT_MS,
): TourTargetState {
  const id = target && target.length > 0 ? selectorId(...target) : null;
  const fallbackId =
    id !== null && fallback && fallback.length > 0
      ? selectorId(...fallback)
      : null;
  const [state, setState] = React.useState<TourTargetState>(() => ({
    rect: null,
    settled: id === null,
    elementId: null,
  }));

  React.useEffect(() => {
    if (id === null) return;
    let element: HTMLElement | null = null;
    let frame = 0;
    let scrolled = false;
    let found = false;
    let resizeObserver: ResizeObserver | null = null;

    const publish = (next: TourTargetState) =>
      setState((current) =>
        current.settled === next.settled &&
        current.elementId === next.elementId &&
        sameRect(current.rect, next.rect)
          ? current
          : next,
      );

    const measure = () => {
      frame = 0;
      const next = pickElement(id, fallbackId);
      if (next !== element) {
        resizeObserver?.disconnect();
        element = next;
        if (element) resizeObserver?.observe(element);
      }
      if (!element) {
        // A target that went away takes its spotlight with it.
        if (found) publish({ rect: null, settled: true, elementId: null });
        return;
      }
      const rect = visibleRect(element);
      if (rect && !scrolled) {
        scrolled = true;
        // On its way into view it is found: the wait must not give up on it
        // while it scrolls.
        found = true;
        const offscreen =
          rect.top < 0 ||
          rect.left < 0 ||
          rect.top + rect.height > window.innerHeight ||
          rect.left + rect.width > window.innerWidth;
        if (offscreen) {
          element.scrollIntoView({
            block: "center",
            inline: "nearest",
            behavior: reduceMotion ? "auto" : "smooth",
          });
          schedule();
          return;
        }
      }
      if (rect) {
        found = true;
        publish({ rect, settled: true, elementId: element.id });
      } else if (found) {
        publish({ rect: null, settled: true, elementId: null });
      }
    };
    const schedule = () => {
      if (frame === 0) frame = window.requestAnimationFrame(measure);
    };

    resizeObserver = new ResizeObserver(schedule);
    const mutationObserver = new MutationObserver(schedule);
    // Attributes too: a closed panel opening is a class change, not a new node.
    mutationObserver.observe(document.body, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "style", "hidden"],
    });
    window.addEventListener("resize", schedule);
    window.addEventListener("scroll", schedule, true);
    // A new target that never shows lets go of the one before it as well.
    const giveUp = window.setTimeout(() => {
      if (!found) publish({ rect: null, settled: true, elementId: null });
    }, waitMs);
    schedule();

    return () => {
      window.clearTimeout(giveUp);
      if (frame !== 0) window.cancelAnimationFrame(frame);
      resizeObserver?.disconnect();
      mutationObserver.disconnect();
      window.removeEventListener("resize", schedule);
      window.removeEventListener("scroll", schedule, true);
    };
  }, [id, fallbackId, reduceMotion, waitMs]);

  return state;
}
