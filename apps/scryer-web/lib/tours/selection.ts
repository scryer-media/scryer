import type {
  TourDefinition,
  TourGate,
  TourGateContext,
  TourStep,
} from "./types.ts";

/** A gate lets a step or tour through only when it answers `true`. */
function passes(gate: TourGate | undefined, context: TourGateContext): boolean {
  return gate ? gate(context) === true : true;
}

/** The server accepts this many tour ids in one progress update. */
export const MAX_TOURS_PER_RECORD = 32;

/** Tour ids the server accepts: 1 to 64 lowercase letters, digits, dots or hyphens. */
const TOUR_ID_PATTERN = /^[a-z0-9][a-z0-9.-]{0,63}$/;

/**
 * The steps of a tour this viewer can use, in order, each pointing at the
 * element this viewer's layout has: on a phone, `mobileTarget` when it has one.
 */
export function visibleSteps(
  tour: TourDefinition,
  context: TourGateContext,
): TourStep[] {
  return tour.steps
    .filter((step) => passes(step.when, context))
    .map((step) =>
      context.isMobile && step.mobileTarget
        ? { ...step, target: step.mobileTarget }
        : step,
    );
}

function isOfferable(tour: TourDefinition, context: TourGateContext): boolean {
  return passes(tour.when, context) && visibleSteps(tour, context).length > 0;
}

/** Orders `0.9.0` before `0.10.0`; a missing release sorts first. */
export function compareReleases(left?: string, right?: string): number {
  const parts = (value?: string) =>
    (value ?? "").split(".").map((part) => Number.parseInt(part, 10) || 0);
  const a = parts(left);
  const b = parts(right);
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    const difference = (a[index] ?? 0) - (b[index] ?? 0);
    if (difference !== 0) return difference;
  }
  return 0;
}

/**
 * The one tour to offer now, or null. Onboarding comes first for an account
 * that has not had it; otherwise the newest unseen highlights tour, so an
 * account that skipped several releases sees one tour, not a queue of them.
 */
export function nextTourToOffer(
  registry: readonly TourDefinition[],
  recorded: ReadonlySet<string>,
  context: TourGateContext,
): TourDefinition | null {
  const unseen = registry.filter(
    (tour) => !recorded.has(tour.id) && isOfferable(tour, context),
  );
  const onboarding = unseen.find((tour) => tour.kind === "onboarding");
  if (onboarding) return onboarding;
  const highlights = unseen
    .filter((tour) => tour.kind === "highlights")
    .sort((left, right) => compareReleases(right.release, left.release));
  return highlights[0] ?? null;
}

/**
 * The tours to record when `tour` ends, finished or skipped. Onboarding also
 * settles every highlights tour, since a new account has nothing to catch up
 * on; a highlights tour also settles the older ones it supersedes.
 */
export function toursSettledBy(
  tour: TourDefinition,
  registry: readonly TourDefinition[],
  recorded: ReadonlySet<string>,
): string[] {
  const settled = registry.filter(
    (candidate) =>
      candidate.id !== tour.id &&
      !recorded.has(candidate.id) &&
      candidate.kind === "highlights" &&
      (tour.kind === "onboarding" ||
        compareReleases(candidate.release, tour.release) <= 0),
  );
  return [tour.id, ...settled.map((candidate) => candidate.id)].slice(
    0,
    MAX_TOURS_PER_RECORD,
  );
}

/**
 * Mistakes in a tour registry, as sentences for a test to print. An empty list
 * means the registry is sound. `hasKey` reports whether a translation exists.
 */
export function registryProblems(
  registry: readonly TourDefinition[],
  hasKey: (key: string) => boolean,
): string[] {
  const problems: string[] = [];
  const ids = new Set<string>();
  let onboardingTours = 0;
  for (const tour of registry) {
    if (!TOUR_ID_PATTERN.test(tour.id)) {
      problems.push(`tour id ${JSON.stringify(tour.id)} is not a valid id`);
    }
    if (ids.has(tour.id)) {
      problems.push(`tour id ${tour.id} is used twice`);
    }
    ids.add(tour.id);
    if (tour.kind === "onboarding") onboardingTours += 1;
    if (tour.kind === "highlights" && !tour.release) {
      problems.push(`highlights tour ${tour.id} names no release`);
    }
    if (tour.steps.length === 0) {
      problems.push(`tour ${tour.id} has no steps`);
    }
    const stepIds = new Set<string>();
    for (const step of tour.steps) {
      if (stepIds.has(step.id)) {
        problems.push(`tour ${tour.id} repeats step ${step.id}`);
      }
      stepIds.add(step.id);
      const keys = [step.titleKey, step.bodyKey];
      if (step.action) keys.push(step.action.labelKey);
      for (const key of keys) {
        if (!hasKey(key)) {
          problems.push(
            `tour ${tour.id} step ${step.id} uses missing key ${key}`,
          );
        }
      }
      if (step.target && step.target.length === 0) {
        problems.push(`tour ${tour.id} step ${step.id} has an empty target`);
      }
      if (step.mobileTarget && step.mobileTarget.length === 0) {
        problems.push(
          `tour ${tour.id} step ${step.id} has an empty mobile target`,
        );
      }
    }
  }
  if (onboardingTours > 1) {
    problems.push(`${onboardingTours} onboarding tours; there is one`);
  }
  return problems;
}
