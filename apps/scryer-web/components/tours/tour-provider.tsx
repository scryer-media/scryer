import * as React from "react";
import { useLocation, useNavigate } from "react-router";
import { useClient } from "urql";
import { toast } from "sonner";
import { useTranslate } from "@/lib/context/translate-context";
import { useUiSettings } from "@/lib/context/ui-settings-context";
import { recordMyUiToursMutation } from "@/lib/graphql/mutations";
import type { AuthUser } from "@/lib/hooks/use-auth";
import {
  ROOT_SHELL_MOBILE_BREAKPOINT,
  useIsMobile,
} from "@/lib/hooks/use-mobile";
import { usePermissions } from "@/lib/hooks/use-permissions";
import { TOUR_REGISTRY } from "@/lib/tours/registry";
import {
  nextTourToOffer,
  toursSettledBy,
  visibleSteps,
} from "@/lib/tours/selection";
import type { TourDefinition, TourGateContext } from "@/lib/tours/types";
import type {
  RecordMyUiToursInput,
  UiSettings,
  UiTourOutcome,
} from "@/lib/types/settings";

// Only an account with a tour to see downloads the overlay.
const TourOverlay = React.lazy(() => import("./tour-overlay"));

type ToursContextValue = {
  /** Opens a tour from its first step, e.g. from a "Replay the tour" menu item. */
  startTour: (tourId: string) => void;
  /** The tour on screen, or null. */
  activeTourId: string | null;
};

const ToursContext = React.createContext<ToursContextValue | null>(null);

export function useTours(): ToursContextValue {
  const value = React.useContext(ToursContext);
  if (!value) throw new Error("useTours must be used within TourProvider");
  return value;
}

/**
 * Offers the guided tours of `registry` to the signed-in account: onboarding
 * to a new account, then each release's highlights once. What the account has
 * finished or skipped is stored on the server, so a tour seen on one device
 * is not offered on another.
 */
export function TourProvider({
  user,
  registry = TOUR_REGISTRY,
  children,
}: {
  user: AuthUser;
  registry?: readonly TourDefinition[];
  children: React.ReactNode;
}) {
  const t = useTranslate();
  const client = useClient();
  const navigate = useNavigate();
  const location = useLocation();
  const { uiSettings, uiSettingsLoaded, setUiSettings } = useUiSettings();
  const permissions = usePermissions(user);
  const isMobile = useIsMobile(ROOT_SHELL_MOBILE_BREAKPOINT);
  const gateContext = React.useMemo<TourGateContext>(
    () => ({ permissions, isMobile }),
    [permissions, isMobile],
  );

  // Tours ended in this page view, ahead of the server's answer, so a slow or
  // failed save never brings a dismissed tour straight back.
  const [endedHere, setEndedHere] = React.useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [replay, setReplay] = React.useState<string | null>(null);
  const [position, setPosition] = React.useState({ tourId: "", index: 0 });

  const recorded = React.useMemo(() => {
    const ids = new Set(endedHere);
    for (const tour of uiSettings.tours) ids.add(tour.tourId);
    return ids;
  }, [endedHere, uiSettings.tours]);

  // Nothing is offered until the account's own settings have loaded: the
  // defaults stand in for a new account and would offer onboarding to anyone.
  const activeTour =
    (replay ? registry.find((tour) => tour.id === replay) : undefined) ??
    (uiSettingsLoaded
      ? nextTourToOffer(registry, recorded, gateContext)
      : null);
  const steps = React.useMemo(
    () => (activeTour ? visibleSteps(activeTour, gateContext) : []),
    [activeTour, gateContext],
  );
  const index =
    activeTour && position.tourId === activeTour.id
      ? Math.min(position.index, steps.length - 1)
      : 0;
  const step = steps[index];

  // A step whose target lives on another page opens that page when the step
  // starts, and only then: the viewer is never pulled back to it afterwards.
  const pathnameRef = React.useRef(location.pathname);
  React.useEffect(() => {
    pathnameRef.current = location.pathname;
  }, [location.pathname]);
  const stepKey = activeTour && step ? `${activeTour.id}/${step.id}` : null;
  const stepRoute = step?.route;
  React.useEffect(() => {
    if (stepKey && stepRoute && pathnameRef.current !== stepRoute) {
      navigate(stepRoute);
    }
  }, [stepKey, stepRoute, navigate]);

  const end = React.useCallback(
    (outcome: UiTourOutcome) => {
      if (!activeTour) return;
      const tourIds = toursSettledBy(activeTour, registry, recorded);
      setEndedHere((current) => new Set([...current, ...tourIds]));
      setReplay(null);
      setPosition({ tourId: "", index: 0 });
      void client
        .mutation<
          { recordMyUiTours?: UiSettings },
          { input: RecordMyUiToursInput }
        >(recordMyUiToursMutation, { input: { tourIds, outcome } })
        .toPromise()
        .then((result) => {
          if (result.data?.recordMyUiTours) {
            setUiSettings(result.data.recordMyUiTours);
          } else {
            toast.error(t("tours.saveFailed"));
          }
        });
    },
    [activeTour, client, recorded, registry, setUiSettings, t],
  );

  const goTo = React.useCallback(
    (nextIndex: number) => {
      if (!activeTour) return;
      setPosition({ tourId: activeTour.id, index: nextIndex });
    },
    [activeTour],
  );

  const contextValue = React.useMemo<ToursContextValue>(
    () => ({
      startTour: (tourId: string) => {
        setPosition({ tourId, index: 0 });
        setReplay(tourId);
      },
      activeTourId: activeTour?.id ?? null,
    }),
    [activeTour],
  );

  return (
    <ToursContext.Provider value={contextValue}>
      {children}
      {activeTour && step ? (
        <React.Suspense fallback={null}>
          <TourOverlay
            key={activeTour.id}
            steps={steps}
            index={index}
            reduceMotion={uiSettings.reduceMotion}
            onBack={() => goTo(Math.max(0, index - 1))}
            onNext={() =>
              index >= steps.length - 1 ? end("COMPLETED") : goTo(index + 1)
            }
            onSkip={() => end("SKIPPED")}
            onAction={(route) => {
              end("COMPLETED");
              navigate(route);
            }}
          />
        </React.Suspense>
      ) : null}
    </ToursContext.Provider>
  );
}
