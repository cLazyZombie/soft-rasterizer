import { defineConfig } from "vite";

export default defineConfig(({ mode }) => ({
  root: ".",
  publicDir: false,
  build: {
    outDir: mode === "test" ? "dist-chapters-test" : "dist",
  },
  preview: {
    host: "127.0.0.1",
    port: 4174,
    strictPort: true,
  },
}));
