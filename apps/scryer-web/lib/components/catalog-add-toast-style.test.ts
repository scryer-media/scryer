import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const card = readFileSync(
  new URL("../../components/root/catalog-add-toast.tsx", import.meta.url),
  "utf8",
);
const toaster = readFileSync(
  new URL("../../components/ui/sonner.tsx", import.meta.url),
  "utf8",
);

test("catalog activity cards retain the standard success border above global CSS", () => {
  const successBorder = "var(--scry-success-border)";
  assert.ok(toaster.includes(`"--success-border": "${successBorder}"`));
  // The unlayered global * border-color rule overrides ordinary Tailwind utilities.
  assert.ok(card.includes(`!border-[${successBorder}]`));
  assert.ok(card.includes("!border-0"), "the outer Sonner frame stays borderless");
});
