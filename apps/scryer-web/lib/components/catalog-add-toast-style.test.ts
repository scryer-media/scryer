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
  assert.ok(
    card.includes("!border-0"),
    "the outer Sonner frame stays borderless",
  );
});

test("every toast variant paints an opaque surface", () => {
  // `!bg-[var(--background)]` is the mechanism: Sonner's richColors sets a translucent
  // background per variant, and only an important utility beats it. Warning and
  // info used to reach for `bg-[linear-gradient(...),var(--scry-bg)]`, which
  // Tailwind emits as `background-image` — a bare colour is not a valid layer
  // there, so the browser dropped the whole declaration and the translucent
  // default showed.
  for (const variant of ["success", "error", "warning", "info"]) {
    assert.ok(
      new RegExp(
        `${variant}:\\s*\\n?\\s*"[^"]*!bg-\\[var\\(--background\\)\\]`,
      ).test(toaster),
      `${variant} toast must force the opaque page surface`,
    );
  }
  assert.ok(
    // The prose above deliberately names the broken shape, so match the literal
    // `0deg` the real utility carried rather than the comment's ellipsis.
    !toaster.includes("bg-[linear-gradient(0deg,"),
    "the invalid background-image layering must not come back",
  );
});
