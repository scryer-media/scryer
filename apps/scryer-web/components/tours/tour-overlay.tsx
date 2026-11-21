import { LifecycleRulesIcon } from "@/components/common/lifecycle-rules-icon";
import * as React from "react";
import { Dialog as DialogPrimitive } from "radix-ui";
import {
  Activity,
  Bell,
  CalendarDays,
  Compass,
  Gavel,
  Library,
  ListChecks,
  Puzzle,
  Search,
  Server,
  Settings,
  Sparkles,
  Users,
  Wand2,
  type LucideIcon,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { useTranslate } from "@/lib/context/translate-context";
import { cn } from "@/lib/utils";
import {
  placeTourCard,
  spotlightRect,
  type TourRect,
  type TourSize,
} from "@/lib/tours/placement";
import { tourArrow, type TourArrow } from "@/lib/tours/arrow";
import type { TourIcon, TourStep, TourTarget } from "@/lib/tours/types";
import { useTourTarget } from "./use-tour-target";

const TOUR_ICONS: Record<TourIcon, LucideIcon> = {
  sparkles: Sparkles,
  search: Search,
  library: Library,
  requests: Wand2,
  activity: Activity,
  calendar: CalendarDays,
  settings: Settings,
  rules: Gavel,
  maintenance: LifecycleRulesIcon,
  lists: ListChecks,
  discover: Compass,
  system: Server,
  users: Users,
  notifications: Bell,
  plugins: Puzzle,
};

export type TourOverlayProps = {
  steps: readonly TourStep[];
  index: number;
  /**
   * Navigation to point at before the step's own target, on the way to the
   * step's page. `onPointed` follows once the arrow has been drawn to it, or
   * at once when it is not on screen.
   */
  pointAt?: TourTarget;
  onPointed?: () => void;
  reduceMotion: boolean;
  onBack: () => void;
  onNext: () => void;
  onSkip: () => void;
  onAction: (route: string) => void;
};

/**
 * One tour step: the page dimmed behind a spotlight on the step's target, and
 * a card in the middle that says what it is. The card moves off the middle
 * only when it would cover the spotlight. Escape skips the tour; the arrow
 * keys step through it. The page underneath is not interactive while a tour
 * is open.
 */
export default function TourOverlay(props: TourOverlayProps) {
  const step = props.steps[props.index];
  if (!step) return null;
  // Keyed by step so each one searches for its own target from scratch.
  return <TourStepOverlay key={step.id} step={step} {...props} />;
}

function TourStepOverlay({
  step,
  steps,
  index,
  pointAt,
  onPointed,
  reduceMotion,
  onBack,
  onNext,
  onSkip,
  onAction,
}: TourOverlayProps & { step: TourStep }) {
  const t = useTranslate();
  const target = pointAt ?? step.target;
  const { rect, settled } = useTourTarget(
    target,
    reduceMotion,
    pointAt ? undefined : step.fallbackTarget,
  );
  // A callback ref: the portal mounts the card after this component does.
  const [card, setCard] = React.useState<HTMLDivElement | null>(null);
  const cardSize = useElementSize(card);
  const viewport = useViewportSize();
  const isLast = index === steps.length - 1;
  const Icon = TOUR_ICONS[step.icon ?? "sparkles"];

  const placement =
    cardSize && settled ? placeTourCard(viewport, cardSize, rect) : null;
  const spot = rect ? spotlightRect(rect) : null;
  // The card fades in where it belongs; only later moves (a target that
  // shifts, a resize) glide, so it never slides in from the corner.
  const placed = placement !== null;
  const [glide, setGlide] = React.useState(false);
  React.useEffect(() => {
    if (!placed) return;
    const frame = window.requestAnimationFrame(() => setGlide(true));
    return () => window.cancelAnimationFrame(frame);
  }, [placed]);

  // On the way to another page: hold on the navigation entry until its arrow
  // is drawn, then go. An entry that is not on screen holds nothing up.
  const pointing = pointAt !== undefined;
  const pointedAt = pointing && settled && (!spot || placed);
  const pointedAtNothing = !spot;
  React.useEffect(() => {
    if (!pointedAt || !onPointed) return;
    if (pointedAtNothing) {
      onPointed();
      return;
    }
    const timer = window.setTimeout(
      onPointed,
      reduceMotion ? POINT_DWELL_REDUCED_MS : POINT_DWELL_MS,
    );
    return () => window.clearTimeout(timer);
  }, [pointedAt, pointedAtNothing, onPointed, reduceMotion]);
  // However the pointing goes, the page changes within a bounded time: a card
  // that cannot be placed never strands the tour on the navigation entry.
  React.useEffect(() => {
    if (!pointing || !onPointed) return;
    const timer = window.setTimeout(onPointed, POINT_HOLD_LIMIT_MS);
    return () => window.clearTimeout(timer);
  }, [pointing, onPointed]);

  return (
    <DialogPrimitive.Root
      open
      onOpenChange={(open) => {
        if (!open) onSkip();
      }}
    >
      <DialogPrimitive.Portal>
        <TourBackdrop
          spot={settled ? spot : null}
          reduceMotion={reduceMotion}
        />
        {placement && cardSize && spot ? (
          <TourArrowLayer
            // Redrawn from the start whenever it points somewhere new.
            key={target?.join("/")}
            arrow={tourArrow(
              { top: placement.top, left: placement.left, ...cardSize },
              spot,
              step.id,
            )}
            reduceMotion={reduceMotion}
          />
        ) : null}
        <DialogPrimitive.Content
          ref={setCard}
          data-slot="tour-card"
          data-testid="guided-tour-dialog"
          data-tour-side={placement?.side}
          onPointerDownOutside={(event) => event.preventDefault()}
          onInteractOutside={(event) => event.preventDefault()}
          onKeyDown={(event) => {
            if (event.key === "ArrowRight") {
              event.preventDefault();
              onNext();
            } else if (event.key === "ArrowLeft" && index > 0) {
              event.preventDefault();
              onBack();
            }
          }}
          className={cn(
            "fixed z-[91] flex w-[min(28rem,calc(100vw-2rem))] flex-col gap-4 rounded-xl border border-border bg-card p-6 text-card-foreground shadow-2xl outline-none",
            !reduceMotion &&
              (glide
                ? "motion-safe:transition-[top,left,opacity]"
                : "motion-safe:transition-opacity") +
                " motion-safe:duration-200 motion-safe:ease-out",
            placement ? "opacity-100" : "opacity-0",
          )}
          style={{ top: placement?.top ?? 0, left: placement?.left ?? 0 }}
        >
          <div className="flex items-start gap-4">
            <span className="grid size-10 shrink-0 place-items-center rounded-lg bg-muted text-primary">
              <Icon className="size-5" aria-hidden="true" />
            </span>
            <div className="min-w-0 space-y-1.5">
              <p className="text-xs font-medium text-muted-foreground">
                {t("tours.stepOf", {
                  current: index + 1,
                  total: steps.length,
                })}
              </p>
              <DialogPrimitive.Title className="text-lg font-semibold leading-snug">
                {t(step.titleKey)}
              </DialogPrimitive.Title>
              <DialogPrimitive.Description className="text-sm leading-relaxed text-muted-foreground">
                {t(step.bodyKey)}
              </DialogPrimitive.Description>
            </div>
          </div>

          <div className="flex items-center gap-1.5" aria-hidden="true">
            {steps.map((dot, dotIndex) => (
              <span
                key={dot.id}
                className={cn(
                  "h-1.5 rounded-full",
                  !reduceMotion && "motion-safe:transition-all",
                  dotIndex === index
                    ? "w-5 bg-primary"
                    : dotIndex < index
                      ? "w-1.5 bg-primary/50"
                      : "w-1.5 bg-muted-foreground/30",
                )}
              />
            ))}
          </div>

          <div className="flex flex-wrap items-center justify-between gap-2">
            <Button
              variant="ghost"
              size="sm"
              className="text-muted-foreground"
              onClick={onSkip}
              data-testid="guided-tour-skip"
            >
              {t("tours.skip")}
            </Button>
            <div className="flex flex-wrap items-center gap-2">
              {index > 0 ? (
                <Button variant="outline" size="sm" onClick={onBack}>
                  {t("tours.back")}
                </Button>
              ) : null}
              {step.action ? (
                <Button
                  variant="secondary"
                  size="sm"
                  onClick={() => onAction(step.action!.route)}
                >
                  {t(step.action.labelKey)}
                </Button>
              ) : null}
              <Button size="sm" onClick={onNext} autoFocus>
                {isLast ? t("tours.done") : t("tours.next")}
              </Button>
            </div>
          </div>
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}

/**
 * Dims the page. With a spotlight, the target shows through a rounded cutout
 * ringed in the highlight color; without one, the page is dimmed and blurred
 * so the card reads as a lightbox.
 */
function TourBackdrop({
  spot,
  reduceMotion,
}: {
  spot: TourRect | null;
  reduceMotion: boolean;
}) {
  const maskId = React.useId();
  if (!spot) {
    return (
      <div
        data-slot="tour-backdrop"
        className="fixed inset-0 z-[90] bg-black/60 backdrop-blur-sm"
      />
    );
  }
  const shape = {
    x: spot.left,
    y: spot.top,
    width: spot.width,
    height: spot.height,
    rx: 10,
  };
  const motion = reduceMotion
    ? undefined
    : "motion-safe:transition-all motion-safe:duration-200 motion-safe:ease-out";
  return (
    <svg
      data-slot="tour-backdrop"
      className="fixed inset-0 z-[90] h-full w-full"
      aria-hidden="true"
    >
      <defs>
        <mask id={maskId}>
          <rect width="100%" height="100%" fill="white" />
          <rect {...shape} className={motion} fill="black" />
        </mask>
      </defs>
      <rect
        width="100%"
        height="100%"
        fill="rgb(0 0 0 / 0.6)"
        mask={`url(#${maskId})`}
      />
      <rect
        {...shape}
        className={cn("stroke-primary", motion)}
        fill="none"
        strokeWidth={2}
      />
    </svg>
  );
}

/** The arrow's head is drawn by about 740 ms; the viewer then sees it a moment. */
const POINT_DWELL_MS = 1100;
/** With motion reduced the arrow shows at once, and still needs reading. */
const POINT_DWELL_REDUCED_MS = 700;
/** The longest a step holds on its navigation before opening its page. */
const POINT_HOLD_LIMIT_MS = 3000;

const ARROW_KEYFRAMES = `
@keyframes tour-arrow-draw { from { stroke-dashoffset: 1; } to { stroke-dashoffset: 0; } }
@media (prefers-reduced-motion: reduce) {
  [data-slot="tour-arrow"] path { animation: none !important; stroke-dashoffset: 0 !important; }
}`;

/**
 * A hand-drawn arrow from the card to the spotlight, sketched in stroke by
 * stroke: the curve, a lighter second pass, then the head. It sits over the
 * dimmed page and under the card, and never takes a click.
 */
function TourArrowLayer({
  arrow,
  reduceMotion,
}: {
  arrow: TourArrow | null;
  reduceMotion: boolean;
}) {
  if (!arrow) return null;
  const stroke = (delayMs: number, durationMs: number) =>
    reduceMotion
      ? undefined
      : {
          strokeDasharray: 1,
          strokeDashoffset: 1,
          animation: `tour-arrow-draw ${durationMs}ms ease-out ${delayMs}ms forwards`,
        };
  return (
    <svg
      data-slot="tour-arrow"
      className="pointer-events-none fixed inset-0 z-[91] h-full w-full text-primary"
      aria-hidden="true"
    >
      {reduceMotion ? null : <style>{ARROW_KEYFRAMES}</style>}
      <g
        fill="none"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        <path
          d={arrow.body}
          pathLength={1}
          strokeWidth={3}
          style={stroke(150, 450)}
        />
        <path
          d={arrow.sketch}
          pathLength={1}
          strokeWidth={1.5}
          opacity={0.5}
          style={stroke(250, 450)}
        />
        <path
          d={arrow.head}
          pathLength={1}
          strokeWidth={3}
          style={stroke(560, 180)}
        />
      </g>
    </svg>
  );
}

function useElementSize(element: HTMLElement | null): TourSize | null {
  const [size, setSize] = React.useState<TourSize | null>(null);
  React.useLayoutEffect(() => {
    if (!element) return;
    const observer = new ResizeObserver(([entry]) => {
      const box = entry.target.getBoundingClientRect();
      setSize((current) =>
        current && current.width === box.width && current.height === box.height
          ? current
          : { width: box.width, height: box.height },
      );
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);
  return size;
}

function useViewportSize(): TourSize {
  return React.useSyncExternalStore(
    subscribeToViewport,
    readViewport,
    readViewport,
  );
}

let viewportSnapshot: TourSize = { width: 0, height: 0 };

function readViewport(): TourSize {
  if (typeof window === "undefined") return viewportSnapshot;
  const { innerWidth: width, innerHeight: height } = window;
  if (viewportSnapshot.width !== width || viewportSnapshot.height !== height) {
    viewportSnapshot = { width, height };
  }
  return viewportSnapshot;
}

function subscribeToViewport(onChange: () => void): () => void {
  window.addEventListener("resize", onChange);
  return () => window.removeEventListener("resize", onChange);
}
