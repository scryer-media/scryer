import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { compression } from "vite-plugin-compression2";
import path from "path";

const DEV_PROXY_TARGET =
  process.env.SCRYER_DEV_PROXY_TARGET?.trim() || "http://127.0.0.1:8080";

const FOUNDATION_CHUNK_MODULES = [
  "/components/common/backend-restart-overlay.tsx",
  "/components/root/global-search-provider.tsx",
  "/lib/context/global-status-context.tsx",
  "/lib/context/search-context.tsx",
  "/lib/context/translate-context.tsx",
  "/lib/hooks/use-global-search.ts",
  "/lib/hooks/use-import-history-subscription.ts",
  "/lib/hooks/use-mobile.ts",
  "/lib/hooks/use-settings-subscription.ts",
  "/lib/utils/download-clients.ts",
  "/lib/utils/formatting.ts",
  "/lib/utils/poster-images.ts",
  "/lib/utils/quality-profiles.ts",
];

const UI_CHUNK_MODULES = [
  "/components/common/confirm-dialog.tsx",
  "/components/common/info-help.tsx",
  "/components/ui/card.tsx",
  "/components/ui/checkbox.tsx",
  "/components/ui/command.tsx",
  "/components/ui/dialog.tsx",
  "/components/ui/hover-card.tsx",
  "/components/ui/input.tsx",
  "/components/ui/label.tsx",
  "/components/ui/select.tsx",
  "/components/ui/separator.tsx",
  "/components/ui/table.tsx",
  "/components/ui/toggle-group.tsx",
  "/lib/utils/action-button-styles.ts",
];

const MEDIA_CHUNK_MODULES = [
  "/components/containers/media-containers.ts",
  "/components/containers/media-content-container.tsx",
  "/components/containers/movie-overview-container.tsx",
  "/components/containers/series-overview-container.tsx",
  "/components/views/overview-back-link.tsx",
];

function matchesChunkModule(id: string, modules: readonly string[]) {
  return modules.some((moduleId) => id.endsWith(moduleId));
}

export default defineConfig(({ mode }) => ({
  base: "./",
  plugins: [
    react({
      babel:
        mode === "production"
          ? { plugins: ["babel-plugin-react-compiler"] }
          : undefined,
    }),
    compression({
      include: /\.(js|css|svg|webmanifest|json)$/i,
      exclude: /service-worker\.js$/,
      algorithms: ["gzip"],
    }),
  ],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "."),
    },
  },
  envPrefix: "SCRYER_",
  build: {
    outDir: "dist",
    sourcemap: false,
    rolldownOptions: {
      output: {
        manualChunks(id: string) {
          // Heavy lazy-loaded libraries — keep isolated behind their lazy() boundary.
          if (id.includes("@codemirror/")) return "vendor-codemirror";

          // Keep related app features together without collapsing the whole route graph.
          if (matchesChunkModule(id, MEDIA_CHUNK_MODULES)) return "app-media";
          if (matchesChunkModule(id, FOUNDATION_CHUNK_MODULES)) return "app-foundation";
          if (matchesChunkModule(id, UI_CHUNK_MODULES)) return "app-ui";

          // Core vendor chunks — loaded on every page.
          if (
            id.includes("/react/") ||
            id.includes("/react-dom/") ||
            id.includes("/react-router") ||
            id.includes("/scheduler/")
          )
            return "vendor-react";

          if (
            id.includes("/urql/") ||
            id.includes("/@urql/") ||
            id.includes("/graphql/") ||
            id.includes("/graphql-ws/")
          )
            return "vendor-graphql";

          // UI primitives — radix, lucide icons, shadcn utilities.
          if (
            id.includes("/radix-ui/") ||
            id.includes("/@radix-ui/") ||
            id.includes("/lucide-react/") ||
            id.includes("/cmdk/") ||
            id.includes("/class-variance-authority/") ||
            id.includes("/clsx/") ||
            id.includes("/tailwind-merge/") ||
            id.includes("/sonner/") ||
            id.includes("/next-themes/")
          )
            return "vendor-ui";
        },
      },
    },
  },
  server: {
    port: 3000,
    host: "0.0.0.0",
    proxy: {
      "/graphql": {
        target: DEV_PROXY_TARGET,
        changeOrigin: true,
        ws: true,
      },
      "/health": {
        target: DEV_PROXY_TARGET,
        changeOrigin: true,
      },
      "/admin": {
        target: DEV_PROXY_TARGET,
        changeOrigin: true,
      },
      "/images": {
        target: DEV_PROXY_TARGET,
        changeOrigin: true,
      },
    },
  },
}));
