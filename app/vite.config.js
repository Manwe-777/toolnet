import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The app holds the owner key: only our own scripts, plus websockets to the
// Nostr relays. Same policy as nginx.conf, as a <meta> for hosts that can't
// set headers (GitHub Pages). frame-ancestors only works as a header.
// Build only: the dev server injects inline scripts.
const CSP =
  "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; " +
  "connect-src 'self' wss: ws:; worker-src 'self'; manifest-src 'self'; base-uri 'none'";

const csp = {
  name: "csp-meta",
  apply: "build",
  transformIndexHtml: () => [
    { tag: "meta", attrs: { "http-equiv": "Content-Security-Policy", content: CSP }, injectTo: "head-prepend" },
    { tag: "meta", attrs: { name: "referrer", content: "no-referrer" }, injectTo: "head-prepend" },
  ],
};

// Relative paths, so the app works from any host and sub path
export default defineConfig({
  base: "./",
  plugins: [react(), csp],
  server: { host: true },
});
