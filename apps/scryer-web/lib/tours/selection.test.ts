import assert from "node:assert/strict";
import test from "node:test";

import en from "../i18n/locales/en.ts";
import { TOUR_REGISTRY } from "./registry.ts";
import {
  MAX_TOURS_PER_RECORD,
  compareReleases,
  nextTourToOffer,
  registryProblems,
  toursSettledBy,
  visibleSteps,
} from "./selection.ts";
import type { TourDefinition, TourGateContext, TourStep } from "./types.ts";

const dictionary = en as Record<string, string>;

function step(id: string, extra: Partial<TourStep> = {}): TourStep {
  return { id, titleKey: "label.back", bodyKey: "label.close", ...extra };
}

const context = {
  isMobile: false,
  permissions: { canRequestMedia: true, canManageConfig: false },
} as unknown as TourGateContext;

const onboarding: TourDefinition = {
  id: "onboarding",
  kind: "onboarding",
  steps: [step("welcome")],
};
const highlights = (
  release: string,
  steps = [step("new")],
): TourDefinition => ({
  id: `release-${release}`,
  kind: "highlights",
  release,
  steps,
});

test("the shipped registry is sound", () => {
  assert.deepEqual(
    registryProblems(TOUR_REGISTRY, (key) => key in dictionary),
    [],
  );
});

test("registry problems name bad ids, duplicates, missing keys and releases", () => {
  const problems = registryProblems(
    [
      { id: "Bad Id", kind: "onboarding", steps: [step("a"), step("a")] },
      {
        id: "dup",
        kind: "highlights",
        steps: [step("b", { titleKey: "tours.absent" })],
      },
      { id: "dup", kind: "onboarding", steps: [] },
    ],
    (key) => key in dictionary,
  );
  assert.deepEqual(problems, [
    'tour id "Bad Id" is not a valid id',
    "tour Bad Id repeats step a",
    "highlights tour dup names no release",
    "tour dup step b uses missing key tours.absent",
    "tour id dup is used twice",
    "tour dup has no steps",
    "2 onboarding tours; there is one",
  ]);
});

test("releases compare numerically", () => {
  assert.ok(compareReleases("0.10.0", "0.9.9") > 0);
  assert.ok(compareReleases("0.23.0", "0.23.0") === 0);
  assert.ok(compareReleases(undefined, "0.1.0") < 0);
});

test("a new account is offered onboarding before any highlights", () => {
  const registry = [highlights("0.23.0"), onboarding];
  assert.equal(nextTourToOffer(registry, new Set(), context)?.id, "onboarding");
});

test("an onboarded account is offered only the newest unseen highlights", () => {
  const registry = [onboarding, highlights("0.9.0"), highlights("0.10.0")];
  assert.equal(
    nextTourToOffer(registry, new Set(["onboarding"]), context)?.id,
    "release-0.10.0",
  );
  assert.equal(
    nextTourToOffer(
      registry,
      new Set(["onboarding", "release-0.10.0", "release-0.9.0"]),
      context,
    ),
    null,
  );
});

test("gated steps and tours are left out, and a tour with no usable step is not offered", () => {
  const configOnly = step("rules", {
    when: ({ permissions }) => permissions.canManageConfig,
  });
  const tour = highlights("0.23.0", [step("open"), configOnly]);
  assert.deepEqual(
    visibleSteps(tour, context).map((visible) => visible.id),
    ["open"],
  );
  const onlyConfig = highlights("0.24.0", [configOnly]);
  const hidden: TourDefinition = { ...highlights("0.25.0"), when: () => false };
  assert.equal(
    nextTourToOffer(
      [tour, onlyConfig, hidden],
      new Set(["onboarding"]),
      context,
    )?.id,
    "release-0.23.0",
  );
});

test("onboarding settles every highlights tour; highlights settle older ones only", () => {
  const registry = [
    onboarding,
    highlights("0.9.0"),
    highlights("0.10.0"),
    highlights("0.11.0"),
  ];
  assert.deepEqual(toursSettledBy(onboarding, registry, new Set()), [
    "onboarding",
    "release-0.9.0",
    "release-0.10.0",
    "release-0.11.0",
  ]);
  assert.deepEqual(
    toursSettledBy(
      registry[2],
      registry,
      new Set(["onboarding", "release-0.9.0"]),
    ),
    ["release-0.10.0"],
  );
});

test("one record never names more tours than the server accepts", () => {
  const many = Array.from({ length: 40 }, (_, index) =>
    highlights(`0.${index}.0`),
  );
  assert.equal(
    toursSettledBy(onboarding, [onboarding, ...many], new Set()).length,
    MAX_TOURS_PER_RECORD,
  );
});

test("a phone spotlights a step's mobile target and keeps the rest", () => {
  const tour: TourDefinition = {
    id: "onboarding",
    kind: "onboarding",
    steps: [
      step("search", { target: ["desktop"], mobileTarget: ["phone"] }),
      step("plain", { target: ["desktop"] }),
    ],
  };
  const targets = (isMobile: boolean) =>
    visibleSteps(tour, { ...context, isMobile }).map((item) => item.target);
  assert.deepEqual(targets(false), [["desktop"], ["desktop"]]);
  assert.deepEqual(targets(true), [["phone"], ["desktop"]]);
});

test("onboarding shows each account only the stops it can open", () => {
  const tour = TOUR_REGISTRY.find((item) => item.kind === "onboarding");
  assert.ok(tour);
  const stepIds = (
    permissions: Record<string, boolean>,
    isMobile = false,
  ): string[] =>
    visibleSteps(tour, {
      isMobile,
      permissions,
    } as unknown as TourGateContext).map((item) => item.id);

  const admin = {
    canViewCatalog: true,
    canRequestMedia: true,
    canManageTitle: true,
    canAccessActivity: true,
    canAccessLists: true,
    canManageSystemSettings: true,
    canManageCatalogSettings: true,
  };
  assert.deepEqual(stepIds(admin), [
    "welcome",
    "search",
    "libraries",
    "discover",
    "lists",
    "activity",
    "indexers",
    "quality-profiles",
    "rules",
    "media-servers",
    "notifications",
    "plugins",
  ]);
  assert.deepEqual(
    stepIds({
      canViewCatalog: true,
      canRequestMedia: true,
      canAccessLists: true,
    }),
    ["welcome", "search", "libraries", "discover", "lists"],
  );
  // On a phone the sidebar stops drop out; the page stops remain.
  assert.deepEqual(stepIds(admin, true), [
    "welcome",
    "search",
    "quality-profiles",
    "plugins",
  ]);
});
