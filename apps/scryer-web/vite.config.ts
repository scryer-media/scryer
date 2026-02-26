import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "."),
    },
  },
  envPrefix: "SCRYER_",
  build: {
    outDir: "dist",
    sourcemap: false,
  },
  server: {
    port: 3000,
    host: "0.0.0.0",
  },
});
