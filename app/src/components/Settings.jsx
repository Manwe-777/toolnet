import { useState } from "react";
import { getPublicKey, npubEncode } from "tool-db";
import { DEFAULT_RELAYS } from "../lib/net.js";
import { exportSecretKey, getRelays, saveRelays } from "../lib/session";
import CopyField from "./CopyField";
import { keyInputProps } from "./Setup";

function Settings({ secretKey, net, onNet, onSignOut, onBack }) {
  const [showKey, setShowKey] = useState(false);
  const [name, setName] = useState(net);
  const [relays, setRelays] = useState(getRelays().join("\n"));

  const applyNet = (e) => {
    e.preventDefault();
    const value = name.trim();
    // Rust puts the name in keys (net-N, gk-N-D): ToolDB keys can't have dots
    if (value && !value.includes(".")) onNet(value);
  };

  const applyRelays = (e) => {
    e.preventDefault();
    const list = relays.split(/\s+/).map((r) => r.trim()).filter((r) => /^wss?:\/\//.test(r));
    if (list.length === 0) return;
    saveRelays(list);
    location.reload();
  };

  return (
    <main className="page">
      <header className="topbar">
        <button className="btn btn--ghost btn--small" onClick={onBack}>← Back</button>
        <h1>Settings</h1>
        <span />
      </header>

      <section className="card">
        <h2>Network</h2>
        <form className="row" onSubmit={applyNet}>
          <input value={name} onChange={(e) => setName(e.target.value)} aria-label="Network name" required {...keyInputProps} />
          <button className="btn btn--small" type="submit">Open</button>
        </form>
        <p className="hint">The <code>--net</code> name the network was created with (default home).</p>
      </section>

      <section className="card">
        <h2>Owner key</h2>
        <p className="muted small">Devices join with <code>toolnet init --owner &lt;this npub&gt;</code>.</p>
        <CopyField value={npubEncode(getPublicKey(secretKey))} />
        {showKey ? (
          <>
            <p className="muted small">Your secret key. Anyone with it controls the network.</p>
            <CopyField value={exportSecretKey(secretKey)} />
          </>
        ) : (
          <button className="btn btn--small" onClick={() => setShowKey(true)}>Show secret key</button>
        )}
      </section>

      <section className="card">
        <h2>Nostr relays</h2>
        <form className="stack" onSubmit={applyRelays}>
          <textarea rows={4} value={relays} onChange={(e) => setRelays(e.target.value)} {...keyInputProps} />
          <p className="hint">One per line. Use relays your devices use (their <code>config.json</code>).</p>
          <div className="row">
            <button className="btn btn--small" type="submit">Save and reconnect</button>
            <button className="btn btn--small btn--ghost" type="button" onClick={() => setRelays(DEFAULT_RELAYS.join("\n"))}>
              Defaults
            </button>
          </div>
        </form>
      </section>

      <button
        className="btn btn--danger btn--wide"
        onClick={() => confirm("Sign out? You'll need the owner key to sign in again.") && onSignOut()}
      >
        Sign out
      </button>
    </main>
  );
}

export default Settings;
