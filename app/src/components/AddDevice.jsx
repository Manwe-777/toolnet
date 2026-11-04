import { useState } from "react";
import { getPublicKey, npubEncode } from "tool-db";
import { parseJoin, nextFreeIp, dnsName } from "../lib/parse.js";
import { approveDevice, DEFAULT_RELAYS } from "../lib/net.js";
import { getRelays, getSecretKey } from "../lib/session.js";
import Scanner from "./Scanner";
import CopyField from "./CopyField";
import { keyInputProps } from "./Setup";

/** Where the Linux installer comes from (a GitHub release) */
const INSTALL_URL = "https://github.com/Manwe-777/toolnet/releases/latest/download/install.sh";

/** The one-liner that installs toolnet on a Linux box and joins this network */
export function linuxInstallCommand(net) {
  const owner = npubEncode(getPublicKey(getSecretKey()));
  const relays = getRelays();
  const custom = relays.join() !== DEFAULT_RELAYS.join();
  const flags = [`--owner ${owner}`, `--net ${net}`, ...(custom ? relays.map((r) => `--relay ${r}`) : [])];
  return `curl -fsSL ${INSTALL_URL} | sh -s -- ${flags.join(" ")}`;
}

/**
 * Approve a device: scan `toolnet id --qr` (or paste), confirm, approve
 */
function AddDevice({ db, net, view, onDone, onCancel }) {
  const [join, setJoin] = useState(null);
  const [pasted, setPasted] = useState("");
  const [scanning, setScanning] = useState(true);
  const [error, setError] = useState("");

  const accept = (text) => {
    try {
      setJoin(parseJoin(text));
      setError("");
      return true;
    } catch (err) {
      setError(err.message);
      return false;
    }
  };

  if (join) {
    return (
      <Confirm
        db={db}
        net={net}
        view={view}
        join={join}
        onDone={onDone}
        onBack={() => setJoin(null)}
      />
    );
  }

  return (
    <main className="page">
      <header className="topbar">
        <button className="btn btn--ghost btn--small" onClick={onCancel}>← Back</button>
        <h1>Add a device</h1>
        <span />
      </header>

      <section className="card">
        <h2>Linux</h2>
        <p className="muted small">
          Run this on the device. It installs toolnet as a service, joins <b>{net}</b> and shows a
          QR code to scan below.
        </p>
        <CopyField value={linuxInstallCommand(net)} wrap />
        <p className="muted small">
          Already installed? Run <code>toolnet id --qr</code> instead.
        </p>
      </section>

      <section className="card">
        <h2>Scan the device's code</h2>
        {scanning ? (
          <Scanner onResult={accept} />
        ) : (
          <button className="btn" onClick={() => setScanning(true)}>Scan a QR code</button>
        )}
        {error && <p className="error">{error}</p>}
      </section>

      <section className="card">
        <h2>Or paste its key</h2>
        <form
          className="stack"
          onSubmit={(e) => {
            e.preventDefault();
            if (accept(pasted)) setScanning(false);
          }}
        >
          <input
            value={pasted}
            onChange={(e) => setPasted(e.target.value)}
            placeholder="npub1... or toolnet:join?..."
            aria-label="Device key"
            required
            {...keyInputProps}
          />
          <button className="btn" type="submit">Continue</button>
        </form>
      </section>
    </main>
  );
}

function Confirm({ db, net, view, join, onDone, onBack }) {
  const existing = view.config.members[join.device];
  const [name, setName] = useState(join.name || "");
  const [ip, setIp] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  let suggestedIp = "";
  try {
    suggestedIp = nextFreeIp(view.config);
  } catch (err) {
    suggestedIp = err.message;
  }

  const approve = async (e) => {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      const done = await approveDevice(db, net, { device: join.device, name, ip: ip.trim() || undefined });
      onDone(done);
    } catch (err) {
      setError(err.message);
      setBusy(false);
    }
  };

  return (
    <main className="page">
      <header className="topbar">
        <button className="btn btn--ghost btn--small" onClick={onBack} disabled={busy}>← Back</button>
        <h1>Approve device</h1>
        <span />
      </header>

      <form className="card stack" onSubmit={approve}>
        <div>
          <div className="label">Device key</div>
          <code className="break">{npubEncode(join.device)}</code>
        </div>
        {existing && (
          <p className="notice notice--bad">
            Already a member as <b>{existing.name}</b> ({existing.ip}). Approving again replaces its
            name, IP and routes.
          </p>
        )}
        <label className="field">
          <span className="label">Name</span>
          <input value={name} onChange={(e) => setName(e.target.value)} placeholder="laptop" required autoFocus {...keyInputProps} />
          <span className="hint">Its MagicDNS name: <code>{dnsName(view.config, { name: name || "name" })}</code></span>
        </label>
        <label className="field">
          <span className="label">Tailnet IP</span>
          <input value={ip} onChange={(e) => setIp(e.target.value)} placeholder={suggestedIp} inputMode="decimal" {...keyInputProps} />
          <span className="hint">Leave empty for the next free one ({suggestedIp}).</span>
        </label>
        {error && <p className="error">{error}</p>}
        <button className="btn btn--primary" type="submit" disabled={busy}>
          {busy ? "Approving…" : `Approve into ${net}`}
        </button>
      </form>
    </main>
  );
}

export default AddDevice;
