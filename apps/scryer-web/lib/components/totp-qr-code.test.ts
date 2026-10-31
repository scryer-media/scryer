import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";

const WEB_ROOT = fileURLToPath(new URL("../..", import.meta.url));
const SCRYER_TOTP_URI =
  "otpauth://totp/Scryer:jen%40example.test?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Scryer&algorithm=SHA1&digits=6&period=30";

async function renderTotpQrCode(value = SCRYER_TOTP_URI): Promise<string> {
  const server = await createServer({
    root: WEB_ROOT,
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
  });

  try {
    const module = await server.ssrLoadModule(
      "/components/common/totp-qr-code.tsx",
    );
    const TotpQrCode = module.TotpQrCode as ComponentType<{ value: string }>;
    return renderToStaticMarkup(createElement(TotpQrCode, { value }));
  } finally {
    await server.close();
  }
}

test("TOTP QR keeps its scanner-safe rendering contract", async () => {
  const markup = await renderTotpQrCode();
  const wrapperClasses = markup.match(/^<div class="([^"]+)">/)?.[1];
  const imageTag = markup.match(/<img\b[^>]*>/)?.[0];

  assert.ok(wrapperClasses);
  assert.ok(imageTag);
  assert.ok(wrapperClasses.split(/\s+/).includes("bg-white"));
  assert.match(imageTag, /src="data:image\/gif;base64,/);
  assert.match(imageTag, /\[image-rendering:pixelated\]/);
  assert.doesNotMatch(markup, /<svg\b/);
});

test("TOTP QR remains sparse enough for 1Password screen scanning", async () => {
  const markup = await renderTotpQrCode(
    "otpauth://totp/Scryer:alexander%2Bmedia%40example.test?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Scryer&algorithm=SHA1&digits=6&period=30",
  );
  const imageTag = markup.match(/<img\b[^>]*>/)?.[0];

  assert.ok(imageTag);
  assert.match(imageTag, /height="424"/);
  assert.match(imageTag, /width="424"/);
});
