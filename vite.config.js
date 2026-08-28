import { defineConfig } from "vite";

export default defineConfig(({ mode }) => ({
  root: "web",
  publicDir: false,
  define: {
    __AUTOMATION__: JSON.stringify(mode === "test"),
  },
  build: {
    outDir: mode === "test" ? "../dist-current-test" : "../dist-current",
    emptyOutDir: true,
  },
  server: {
    host: "127.0.0.1",
    port: 5173,
    strictPort: true,
  },
  preview: {
    host: "127.0.0.1",
    port: 4173,
    strictPort: true,
  },
}));
