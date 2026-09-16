import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createElement, type ComponentType, type Context } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { createServer } from "vite";
import type { Translate } from "@/components/root/types";
import type { SettingsProxiesSectionProps } from "@/components/views/settings/settings-proxies-section";
import { PROXY_INITIAL_DRAFT } from "../types/proxies.ts";

async function withProxySection(
  run: (
    props: SettingsProxiesSectionProps,
    render: () => string,
  ) => void,
) {
  const server = await createServer({
    root: fileURLToPath(new URL("../..", import.meta.url)),
    server: { middlewareMode: true },
    appType: "custom",
    logLevel: "silent",
    plugins: [{
      name: "proxy-form-date-format-fixture",
      enforce: "pre",
      load(id) {
        if (id.endsWith("/lib/context/ui-settings-context.tsx")) {
          return 'export function useUiDateTimeFormat() { return "LOCALE"; }';
        }
      },
    }],
  });
  try {
    const [view, translation] = await Promise.all([
      server.ssrLoadModule("/components/views/settings/settings-proxies-section.tsx"),
      server.ssrLoadModule("/lib/context/translate-context.tsx"),
    ]);
    const Component = view.SettingsProxiesSection as ComponentType<SettingsProxiesSectionProps>;
    const TranslateContext = translation.TranslateContext as Context<Translate | null>;
    const props: SettingsProxiesSectionProps = {
      proxyConfigs: [],
      proxyDraft: { ...PROXY_INITIAL_DRAFT, providerType: "ssh_tunnel" },
      setProxyDraft: () => {},
      editingProxyId: null,
      isProxyEditorOpen: true,
      mutatingProxyId: null,
      testingProxyId: null,
      resettingHostKeyProxyId: null,
      submitProxy: () => {},
      resetProxyDraft: () => {},
      startCreateProxy: () => {},
      changeProxyProvider: () => {},
      editProxy: () => {},
      importWireguardConfig: () => false,
      testProxy: () => {},
      deleteProxy: () => {},
      requestResetHostKey: () => {},
      copyTunnelPublicKey: () => {},
    };
    const render = () => renderToStaticMarkup(
      createElement(TranslateContext.Provider, { value: (key) => key },
        createElement(Component, props)),
    );
    run(props, render);
  } finally {
    await server.close();
  }
}

test("SSH editor requires a key and never offers password or key removal controls", async () => {
  await withProxySection((props, render) => {
    const fresh = render();
    assert.doesNotMatch(fresh, /id="settings-indexer-proxy-password"/);
    assert.match(fresh, /<textarea[^>]*id="settings-indexer-proxy-private-key"[^>]*required=""/);

    props.proxyDraft.hasStoredPrivateKey = true;
    props.proxyDraft.hasStoredCredentials = true;
    const saved = render();
    assert.doesNotMatch(saved, /id="settings-indexer-proxy-(?:password|clear-password|clear-private-key)"/);
    assert.doesNotMatch(saved, /<textarea[^>]*id="settings-indexer-proxy-private-key"[^>]*required=""/);

    props.proxyDraft.providerType = "http3";
    const http3 = render();
    assert.match(http3, /id="settings-indexer-proxy-password"/);
    assert.match(http3, /id="settings-indexer-proxy-clear-credentials"/);
    assert.doesNotMatch(http3, /id="settings-indexer-proxy-private-key"/);
    assert.doesNotMatch(http3, /settings.proxyTunnelAuthHelp/);

    props.proxyDraft.providerType = "http";
    assert.match(render(), /id="settings-indexer-proxy-password"/);
  });
});

test("proxy editor offers cancel while creating, editing, and importing a WireGuard config", async () => {
  await withProxySection((props, render) => {
    const creating = render();
    assert.match(creating, /id="settings-indexer-proxy-save"/);
    assert.match(creating, /id="settings-indexer-proxy-cancel"/);

    props.editingProxyId = "proxy-1";
    assert.match(render(), /id="settings-indexer-proxy-cancel"/);

    props.editingProxyId = null;
    props.proxyDraft.providerType = "wireguard";
    const importing = render();
    assert.match(importing, /id="settings-indexer-proxy-import-config"/);
    assert.doesNotMatch(importing, /id="settings-indexer-proxy-save"/);
    assert.match(importing, /id="settings-indexer-proxy-cancel"/);
  });
});
