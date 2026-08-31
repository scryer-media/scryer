import { defineConfig } from "vite";
import babel from "@rolldown/plugin-babel";
import react, { reactCompilerPreset } from "@vitejs/plugin-react";
import { constants as zlibConstants } from "node:zlib";
import { compression, defineAlgorithm } from "vite-plugin-compression2";

const DEV_PROXY_TARGET =
  process.env.SCRYER_DEV_PROXY_TARGET?.trim() || "http://127.0.0.1:8080";
const DEV_WATCH_USE_POLLING =
  process.env.SCRYER_VITE_USE_POLLING?.trim() === "true";
const DEV_WATCH_POLL_INTERVAL = Number.parseInt(
  process.env.SCRYER_VITE_POLL_INTERVAL_MS?.trim() || "250",
  10,
);

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
  "/components/ui/input.tsx",
  "/components/ui/label.tsx",
  "/components/ui/select.tsx",
  "/components/ui/separator.tsx",
  "/components/ui/table.tsx",
  "/components/ui/toggle-group.tsx",
  "/lib/utils/action-button-styles.ts",
];

const MEDIA_CHUNK_MODULES = [
  "/components/containers/media-content-container.tsx",
  "/components/containers/series-overview-container.tsx",
  "/components/views/overview-back-link.tsx",
];

function matchesChunkModule(id: string, modules: readonly string[]) {
  return modules.some((moduleId) => id.endsWith(moduleId));
}

export default defineConfig(({ command, mode }) => ({
  base: command === "serve" ? "/" : "./",
  plugins: [
    react(),
    ...(mode === "production"
      ? [babel({ presets: [reactCompilerPreset()] })]
      : []),
    compression({
      include: /\.(js|css|svg|webmanifest|json)$/i,
      exclude: /service-worker\.js$/,
      algorithms: [
        defineAlgorithm("brotliCompress", {
          params: {
            [zlibConstants.BROTLI_PARAM_QUALITY]: 11,
          },
        }),
      ],
      skipIfLargerOrEqual: false,
    }),
  ],
  resolve: {
    alias: {
      "@": import.meta.dirname,
    },
  },
  // This dependency is first reached through lazy media routes. Prebundle it at
  // startup so Vite does not replace its optimized URL while a route is loading.
  optimizeDeps: {
    include: ["@tanstack/react-virtual"],
  },
  envPrefix: "SCRYER_",
  build: {
    target: "es2022",
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
    watch: DEV_WATCH_USE_POLLING
      ? {
          usePolling: true,
          interval: Number.isFinite(DEV_WATCH_POLL_INTERVAL)
            ? DEV_WATCH_POLL_INTERVAL
            : 250,
        }
      : undefined,
    proxy: {
      "/graphql": {
        target: DEV_PROXY_TARGET,
        changeOrigin: true,
        ws: true,
      },
      "/authless-client": {
        target: DEV_PROXY_TARGET,
        changeOrigin: true,
      },
      "/.well-known/oauth-authorization-server": {
        target: DEV_PROXY_TARGET,
        changeOrigin: false,
      },
      "/oauth/authorize/decision": {
        target: DEV_PROXY_TARGET,
        changeOrigin: true,
      },
      "/oauth/token": {
        target: DEV_PROXY_TARGET,
        changeOrigin: true,
      },
      "/oauth/revoke": {
        target: DEV_PROXY_TARGET,
        changeOrigin: true,
      },
      "/health": {
        target: DEV_PROXY_TARGET,
        changeOrigin: true,
      },
      "/admin": {
        target: DEV_PROXY_TARGET,
        changeOrigin: true,
      },
      "/backups": {
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
