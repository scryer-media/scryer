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
    manifest: true,
    sourcemap: false,
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
