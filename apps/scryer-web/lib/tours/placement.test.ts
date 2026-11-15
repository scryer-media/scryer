import assert from "node:assert/strict";
import test from "node:test";

import {
  TOUR_TARGET_GAP,
  TOUR_VIEWPORT_MARGIN,
  placeTourCard,
  spotlightRect,
} from "./placement.ts";

const viewport = { width: 1440, height: 900 };
const card = { width: 420, height: 240 };

test("without a target the card is a centered lightbox", () => {
  assert.deepEqual(placeTourCard(viewport, card, null), {
    side: "center",
    left: 510,
    top: 330,
  });
});

test("a target clear of the middle leaves the card centered", () => {
  const sidebarItem = { top: 120, left: 16, width: 220, height: 36 };
  assert.equal(placeTourCard(viewport, card, sidebarItem).side, "center");
});

test("a target behind the middle moves the card to the roomier side", () => {
  const middleTarget = { top: 300, left: 600, width: 240, height: 60 };
  const placement = placeTourCard(viewport, card, middleTarget);
  const spot = spotlightRect(middleTarget);
  assert.equal(placement.side, "below");
  assert.equal(placement.top, spot.top + spot.height + TOUR_TARGET_GAP);
  assert.equal(placement.left, 510);
});

test("a tall centered target sends the card beside it", () => {
  const tall = { top: 40, left: 520, width: 300, height: 820 };
  const placement = placeTourCard(viewport, card, tall);
  assert.equal(placement.side, "right");
  assert.ok(
    placement.left + card.width <= viewport.width - TOUR_VIEWPORT_MARGIN,
  );
});

test("with no room on any side the card stays centered over the page", () => {
  const phone = { width: 375, height: 700 };
  const everything = { top: 10, left: 10, width: 355, height: 680 };
  assert.equal(
    placeTourCard(phone, { width: 343, height: 260 }, everything).side,
    "center",
  );
});

test("a card wider than a phone keeps its margin", () => {
  const phone = { width: 375, height: 700 };
  const placement = placeTourCard(phone, { width: 343, height: 260 }, null);
  assert.equal(placement.left, TOUR_VIEWPORT_MARGIN);
});
