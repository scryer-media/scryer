import { useEffect, useRef, useState, type CSSProperties, type RefObject } from "react";
import { createPortal } from "react-dom";
import { PageShellFallback } from "@/components/root/page-shell-fallback";
import { cn } from "@/lib/utils";

const LOGO_SRC = `${import.meta.env.BASE_URL}scryer-logo.svg`;

/**
 * How long the logo takes to appear and settle in the middle of the screen
 * before it may fly: the first 48% of Weaver's 1.2s welcome.
 */
const APPEAR_AND_HOLD_MS = 576;

/** When the logo first appeared, shared by every copy of it (see `SetupIntroLogo`). */
let appearedAt: number | null = null;

function msSinceAppeared() {
  appearedAt ??= performance.now();
  return performance.now() - appearedAt;
}

export function prefersReducedMotion() {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true;
}

/** Whether this visit to setup gets the welcome: not when reopened from Settings. */
function setupWelcomes() {
  return new URLSearchParams(window.location.search).get("reentry") !== "1";
}

/**
 * The large logo in the middle of the screen that opens setup, the same welcome
 * Weaver's first-run setup plays.
 *
 * It stands in for the loading screens while setup's code and sign-in load, so
 * it is drawn in turn by the route's loading fallback, the setup page and the
 * wizard. Each copy picks the appearing animation up where the last left off,
 * so it reads as one logo. Given a `flight`, it flies from the middle and
 * shrinks onto the wizard's header logo, then calls `onLanded`.
 */
function SetupIntroLogo({
  flight,
  flyingRef,
  onLanded,
}: {
  flight?: CSSProperties | null;
  flyingRef?: RefObject<HTMLImageElement | null>;
  onLanded?: () => void;
}) {
  const [appearDelay] = useState(() => `${-msSinceAppeared()}ms`);

  return createPortal(
    <div
      aria-hidden="true"
      className="pointer-events-none fixed inset-0 z-50 flex items-center justify-center"
    >
      <img
        ref={flyingRef}
        src={LOGO_SRC}
        alt=""
        draggable={false}
        style={flight ?? { animationDelay: appearDelay }}
        className={cn(
          "size-[min(56vmin,420px)] select-none object-contain",
          flight ? "setup-intro-fly" : "setup-intro-appear",
        )}
        onAnimationEnd={(event) => {
          if (event.animationName === "setup-intro-fly") onLanded?.();
        }}
      />
    </div>,
    document.body,
  );
}

/**
 * The page setup shows while its code or sign-in loads: the welcome's logo on
 * setup's background, or the ordinary loading screen when setup is reopened
 * from Settings.
 */
export function SetupIntroLoading() {
  if (!setupWelcomes()) {
    return <PageShellFallback />;
  }
  return (
    <div className="min-h-screen bg-fixed [background-image:var(--scry-shell-bg)]">
      <SetupIntroLogo />
    </div>
  );
}

/** Whether the wizard plays the welcome's flight when it mounts. */
export function setupIntroFlies() {
  return setupWelcomes() && !prefersReducedMotion();
}

/**
 * How far the big centred logo has to travel and shrink to land exactly on
 * `target`. The offsets ignore the logo's own animations, so this holds
 * whether it is still appearing or already flying.
 */
function aimAt(flying: HTMLImageElement, target: HTMLElement) {
  const landing = target.getBoundingClientRect();
  const startX = flying.offsetLeft + flying.offsetWidth / 2;
  const startY = flying.offsetTop + flying.offsetHeight / 2;
  return {
    "--setup-intro-x": `${landing.left + landing.width / 2 - startX}px`,
    "--setup-intro-y": `${landing.top + landing.height / 2 - startY}px`,
    "--setup-intro-scale": String(landing.height / flying.offsetHeight),
  };
}

/**
 * The welcome's logo inside the wizard. It holds in the middle until `ready`
 * and the artwork has decoded, then flies onto the header logo at `targetRef`.
 *
 * The wizard hides its header logo while this shows (`setup-intro` on its
 * shell) and starts the page's own animations when `onStart` fires with how
 * long the flight waits to begin (`setup-intro-playing`), so both run
 * together. The wizard's page is centred, so the header only settles once the
 * page's content has; that is what `ready` waits for. `onDone` fires when the
 * logo has landed, or at once if there is no header logo to land on. Pass
 * stable callbacks: a new one restarts the wait.
 */
export function SetupIntroMark({
  targetRef,
  ready,
  onStart,
  onDone,
}: {
  targetRef: RefObject<HTMLElement | null>;
  ready: boolean;
  onStart: (flightDelayMs: number) => void;
  onDone: () => void;
}) {
  const flyingRef = useRef<HTMLImageElement>(null);
  const [flight, setFlight] = useState<CSSProperties | null>(null);

  useEffect(() => {
    const flying = flyingRef.current;
    if (!ready || !flying) {
      return;
    }
    let cancelled = false;
    let frame = 0;
    void flying
      .decode()
      .catch(() => undefined)
      .then(() => {
        if (cancelled) return;
        const target = targetRef.current;
        if (!target) {
          onDone();
          return;
        }
        const delayMs = Math.max(0, APPEAR_AND_HOLD_MS - msSinceAppeared());
        setFlight({
          ...aimAt(flying, target),
          animationDelay: `${delayMs}ms`,
        } as CSSProperties);
        onStart(delayMs);

        // The header can still move while the logo flies: a step opened by
        // a refresh fills in as its data loads, and a taller page pushes the
        // centred header up. Re-aim every frame so it lands where the header
        // logo is when it arrives.
        const track = () => {
          const current = targetRef.current;
          if (current) {
            for (const [name, value] of Object.entries(aimAt(flying, current))) {
              flying.style.setProperty(name, value);
            }
          }
          frame = requestAnimationFrame(track);
        };
        frame = requestAnimationFrame(track);
      });
    return () => {
      cancelled = true;
      cancelAnimationFrame(frame);
    };
  }, [ready, targetRef, onStart, onDone]);

  return <SetupIntroLogo flight={flight} flyingRef={flyingRef} onLanded={onDone} />;
}
