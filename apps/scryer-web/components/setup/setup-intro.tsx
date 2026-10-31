import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type RefObject,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { cn } from "@/lib/utils";

const LOGO_SRC = `${import.meta.env.BASE_URL}scryer-logo.svg`;

/**
 * How long the logo takes to appear and settle in the middle of the screen
 * before it may fly: the first 48% of Weaver's 1.2s welcome.
 */
const APPEAR_AND_HOLD_MS = 576;

const SetupIntroContext = createContext<
  ((target: HTMLElement | null) => void) | null
>(null);

/** One load-in owner above the authentication decision and the wizard routes. */
export function SetupIntroBoundary({ children }: { children: ReactNode }) {
  const [intro, setIntro] = useState(setupIntroFlies);
  const [ready, setReady] = useState(false);
  const targetRef = useRef<HTMLElement | null>(null);
  const register = useCallback((target: HTMLElement | null) => {
    targetRef.current = target;
    setReady(true);
  }, []);
  const finish = useCallback(() => setIntro(false), []);
  return (
    <SetupIntroContext.Provider value={register}>
      <div inert={intro} style={intro ? { visibility: "hidden" } : undefined}>
        {children}
      </div>
      {intro && (
        <SetupIntroMark targetRef={targetRef} ready={ready} onDone={finish} />
      )}
    </SetupIntroContext.Provider>
  );
}

/** Screens supply a landing target; they never start or replay the splash. */
export function useSetupIntroTarget<T extends HTMLElement>(ready = true) {
  const register = useContext(SetupIntroContext);
  const target = useRef<T>(null);
  useEffect(() => {
    if (ready) register?.(target.current);
  }, [ready, register]);
  return target;
}

export function prefersReducedMotion() {
  return (
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches === true
  );
}

/** Whether this visit to setup gets the welcome: not when reopened from Settings. */
function setupWelcomes() {
  return new URLSearchParams(window.location.search).get("reentry") !== "1";
}

/**
 * The large logo in the middle of the screen that opens setup, the same welcome
 * Weaver's first-run setup plays.
 *
 * The shared boundary keeps this one logo mounted while it chooses and loads
 * the first screen, then flies it onto that screen's header.
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
        style={flight ?? undefined}
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

/** Route loading keeps the background; only the boundary renders a splash. */
export function SetupIntroLoading() {
  return (
    <div className="min-h-screen bg-fixed [background-image:var(--scry-shell-bg)]" />
  );
}

/** Whether this page load animates the shared welcome. */
function setupIntroFlies() {
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
 * The shared welcome holds in the middle until the chosen screen is `ready`
 * and the artwork has decoded, then flies onto the header logo at `targetRef`.
 *
 * The boundary hides the chosen screen until the logo lands. A null target
 * dismisses the load-in for routes outside setup.
 */
function SetupIntroMark({
  targetRef,
  ready,
  onDone,
}: {
  targetRef: RefObject<HTMLElement | null>;
  ready: boolean;
  onDone: () => void;
}) {
  const flyingRef = useRef<HTMLImageElement>(null);
  const [appearedAt] = useState(() => performance.now());
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
        const delayMs = Math.max(
          0,
          APPEAR_AND_HOLD_MS - (performance.now() - appearedAt),
        );
        setFlight({
          ...aimAt(flying, target),
          animationDelay: `${delayMs}ms`,
        } as CSSProperties);

        // The header can still move while the logo flies: a step opened by
        // a refresh fills in as its data loads, and a taller page pushes the
        // centred header up. Re-aim every frame so it lands where the header
        // logo is when it arrives.
        const track = () => {
          const current = targetRef.current;
          if (current) {
            for (const [name, value] of Object.entries(
              aimAt(flying, current),
            )) {
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
  }, [ready, targetRef, onDone, appearedAt]);

  return (
    <SetupIntroLogo flight={flight} flyingRef={flyingRef} onLanded={onDone} />
  );
}
