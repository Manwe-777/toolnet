/**
 * "5 min ago", from seconds (Rust's timestamps)
 */
export function ago(seconds, now = Date.now() / 1000) {
  if (!seconds) return "never";
  const s = Math.max(0, Math.round(now - seconds));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
}

/**
 * Node info is only republished when a device's addresses (or the epoch)
 * change, so its age is when we last heard from it, not whether it's up now
 */
export const RECENT_SECONDS = 15 * 60;
export const isRecent = (seconds, now = Date.now() / 1000) => !!seconds && now - seconds < RECENT_SECONDS;
