import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Relative paths, so the app works from any host and sub path
export default defineConfig({
  base: "./",
  plugins: [react()],
  server: { host: true },
});
