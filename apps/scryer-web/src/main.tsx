import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "react-router/dom";
import { Provider as UrqlProvider } from "urql";
import { ThemeProvider } from "next-themes";
import { backendClient } from "@/lib/graphql/urql-client";
import { SELECTABLE_THEMES } from "@/lib/theme";
import { UiSettingsProvider } from "@/lib/context/ui-settings-context";
import { InstanceFeaturesProvider } from "@/lib/context/instance-features-context";
import { PageShellFallback } from "@/components/root/page-shell-fallback";
import { URL_PARAM_LANGUAGE } from "@/lib/constants/settings";
import { loadLocaleDictionary } from "@/lib/i18n";
import { readStoredLanguageCode } from "@/lib/hooks/use-language";
import { parseLanguageFromParam } from "@/lib/utils/routing";
import { claimViteImportRecovery } from "@/lib/utils/vite-import-recovery";

import "@fontsource-variable/inter";
import "@fontsource-variable/space-grotesk";

import "@/app/globals.css";

import { registerServiceWorker } from "@/lib/pwa/register-service-worker";
import { router } from "./router";

type VitePreloadErrorEvent = Event & { payload?: unknown };

if (import.meta.env.PROD) {
  window.addEventListener("vite:preloadError", (event) => {
    const preloadEvent = event as VitePreloadErrorEvent;
    if (!claimViteImportRecovery(preloadEvent.payload, window.sessionStorage)) {
      return;
    }
    event.preventDefault();
    window.location.reload();
  });
}

function initialUiLanguage() {
  const searchParams = new URLSearchParams(window.location.search);
  return (
    parseLanguageFromParam(searchParams.get(URL_PARAM_LANGUAGE)) ??
    readStoredLanguageCode()
  );
}

const root = createRoot(document.getElementById("root")!);
root.render(<PageShellFallback />);

async function bootstrap() {
  try {
    await loadLocaleDictionary(initialUiLanguage());
  } catch (error) {
    console.error("Failed to load the initial translation dictionary.", error);
  }

  root.render(
    <StrictMode>
      <ThemeProvider attribute="class" defaultTheme="dark" enableSystem themes={[...SELECTABLE_THEMES]}>
        <UrqlProvider value={backendClient}>
          <UiSettingsProvider>
            <InstanceFeaturesProvider>
              <RouterProvider router={router} />
            </InstanceFeaturesProvider>
          </UiSettingsProvider>
        </UrqlProvider>
      </ThemeProvider>
    </StrictMode>,
  );

  registerServiceWorker();
}

void bootstrap();
