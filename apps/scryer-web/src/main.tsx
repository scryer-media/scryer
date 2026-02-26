import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "react-router-dom";
import { Provider as UrqlProvider } from "urql";
import { ThemeProvider } from "next-themes";
import { Toaster } from "@/components/ui/sonner";
import { backendClient } from "@/lib/graphql/urql-client";

import "@fontsource/inter/latin-400.css";
import "@fontsource/inter/latin-500.css";
import "@fontsource/inter/latin-600.css";
import "@fontsource/inter/latin-700.css";
import "@fontsource/space-grotesk/latin-400.css";
import "@fontsource/space-grotesk/latin-500.css";
import "@fontsource/space-grotesk/latin-600.css";
import "@fontsource/space-grotesk/latin-700.css";

import "@/app/globals.css";

import { router } from "./router";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ThemeProvider attribute="class" defaultTheme="dark" enableSystem>
      <UrqlProvider value={backendClient}>
        <RouterProvider router={router} />
        <Toaster position="top-right" duration={10000} />
      </UrqlProvider>
    </ThemeProvider>
  </StrictMode>,
);
