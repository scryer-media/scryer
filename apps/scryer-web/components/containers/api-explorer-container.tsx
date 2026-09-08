import { useEffect, useMemo, useState } from "react";
import { GraphiQL } from "graphiql";
import "graphiql/setup-workers/vite";
import "graphiql/style.css";
import { useTheme } from "next-themes";
import { useTranslate } from "@/lib/context/translate-context";
import { useRefreshInstanceFeatures } from "@/lib/context/instance-features-context";
import { SingleSelectField } from "@/components/ui/select";
import { ViewLoadingFallback } from "@/components/common/view-loading-fallback";
import { getAuthToken } from "@/lib/hooks/use-auth";
import { getAuthlessWebClientProof } from "@/lib/authless-web-client";
import { getRuntimeGraphqlUrl } from "@/lib/runtime-config";
import { scryerFetch } from "@/lib/graphql/urql-client";
import { createApiExplorerTransport, type ApiExplorerMode } from "@/lib/graphql/api-explorer-transport";

type ExplorerTransport = ReturnType<typeof createApiExplorerTransport>;

function Editor({ mode }: { mode: ApiExplorerMode }) {
  const { resolvedTheme } = useTheme();
  const refresh = useRefreshInstanceFeatures();
  const [transport, setTransport] = useState<ExplorerTransport | null>(null);
  // Keep query history within this editor session, including any sensitive variables.
  const storage = useMemo(() => {
    const values = new Map<string, string>();
    return {
      get length() { return values.size; },
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => { values.set(key, value); },
      removeItem: (key: string) => { values.delete(key); },
      clear: () => values.clear(),
    };
  }, []);
  useEffect(() => {
    const next = createApiExplorerTransport({
      mode,
      url: new URL(getRuntimeGraphqlUrl(), window.location.origin).toString(),
      fetch: scryerFetch,
      getToken: getAuthToken,
      getProof: getAuthlessWebClientProof,
      onUnavailable: () => { void refresh(); },
    });
    setTransport(next);
    return () => next.dispose();
  }, [mode, refresh]);
  if (!transport) return <ViewLoadingFallback />;
  return (
    <GraphiQL
      fetcher={transport.fetcher}
      storage={storage}
      shouldPersistHeaders={false}
      showPersistHeadersSettings={false}
      forcedTheme={resolvedTheme === "dark" ? "dark" : "light"}
      defaultQuery={"query {\n  scryerVersion\n}"}
      defaultEditorToolsVisibility="variables"
    />
  );
}

export default function ApiExplorerContainer() {
  const t = useTranslate();
  const [mode, setMode] = useState<ApiExplorerMode>("api-key");
  return (
    <section className="flex h-[calc(100dvh-10rem)] min-h-[32rem] flex-col gap-3" aria-label="API explorer">
      <div className="flex flex-wrap items-end gap-4">
        <SingleSelectField
          label={t("apiExplorer.accessMode")}
          value={mode}
          onValueChange={(value) => { if (value === "api-key" || value === "oauth") setMode(value); }}
          options={[
            { value: "api-key", label: t("apiExplorer.apiKey") },
            { value: "oauth", label: t("apiExplorer.oauth") },
          ]}
          triggerClassName="w-48"
        />
        <div className="pb-1 text-sm text-muted-foreground">
          <p>{t(mode === "api-key" ? "apiExplorer.apiKeyHelp" : "apiExplorer.oauthHelp")}</p>
          <p>{t("apiExplorer.sessionHelp")}</p>
        </div>
      </div>
      <div className="min-h-0 flex-1 overflow-hidden rounded-xl border border-border">
        <Editor key={mode} mode={mode} />
      </div>
    </section>
  );
}
