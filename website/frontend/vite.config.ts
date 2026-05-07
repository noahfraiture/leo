import { resolve } from "node:path";

import tailwindcss from "@tailwindcss/vite";
import solid from "vite-plugin-solid";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [solid(), tailwindcss()],
  build: {
    manifest: "manifest.json",
    outDir: "dist",
    rollupOptions: {
      input: resolve(__dirname, "src/main.ts"),
    },
  },
  test: {
    environment: "jsdom",
    globals: true,
  },
});
