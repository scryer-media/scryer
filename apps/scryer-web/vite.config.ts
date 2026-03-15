import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { compression } from "vite-plugin-compression2";
import path from "path";

export default defineConfig({
  base: "./",
  plugins: [
    react({
      babel: {
        plugins: ["babel-plugin-react-compiler"],
      },
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
        manualChunks(id) {
          // Heavy lazy-loaded libraries — keep isolated behind their lazy() boundary.
          if (id.includes("@fullcalendar/")) return "vendor-calendar";
          if (id.includes("@codemirror/")) return "vendor-codemirror";

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
  },
});
