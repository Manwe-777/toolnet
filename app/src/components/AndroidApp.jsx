import { useEffect, useState } from "react";

/**
 * The Android app, served next to this app as download/toolnet.apk and
 * download/version.json (the Pages deploy copies them from the newest
 * release). Shows nothing when there isn't one.
 */
function AndroidApp() {
  const [info, setInfo] = useState(null);

  useEffect(() => {
    let cancelled = false;
    fetch("./download/version.json", { cache: "no-cache" })
      .then((r) => (r.ok ? r.json() : null))
      .then((v) => !cancelled && v?.version && setInfo(v))
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  if (!info) return null;
  const mb = (info.size / 1e6).toFixed(1);
  return (
    <section className="card">
      <h2>Android app</h2>
      <p className="muted small">
        Puts this phone on the network: the homelab, its LAN and the exit node from anywhere.
        Install it, set it up with this network, then approve it here with its join code.
      </p>
      <a className="btn btn--primary btn--wide" href="./download/toolnet.apk" download="toolnet.apk">
        Download toolnet {info.version} ({mb} MB)
      </a>
      <p className="hint">
        Android asks to allow installing apps from your browser the first time. SHA-256{" "}
        <code title={info.sha256}>{info.sha256.slice(0, 16)}…</code>
      </p>
    </section>
  );
}

export default AndroidApp;
