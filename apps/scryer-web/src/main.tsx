import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "react-router-dom";
import { Provider as UrqlProvider } from "urql";
import { ThemeProvider } from "next-themes";
import { Toaster } from "@/components/ui/sonner";
import { backendClient } from "@/lib/graphql/urql-client";
import { SELECTABLE_THEMES } from "@/lib/theme";

import "@fontsource/inter/latin-400.css";
import "@fontsource/inter/latin-600.css";
import "@fontsource/space-grotesk/latin-600.css";

import "@/app/globals.css";

import { registerServiceWorker } from "@/lib/pwa/register-service-worker";
import { router } from "./router";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ThemeProvider attribute="class" defaultTheme="dark" enableSystem themes={[...SELECTABLE_THEMES]}>
      <UrqlProvider value={backendClient}>
        <RouterProvider router={router} />
        <Toaster position="top-right" duration={10000} />
      </UrqlProvider>
    </ThemeProvider>
  </StrictMode>,
);

registerServiceWorker();

// Defer non-critical font weights
import("@/lib/fonts/deferred-fonts").then((m) => m.loadDeferredFonts());
