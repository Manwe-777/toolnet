import { useEffect, useState } from "react";
import { createNetwork, listNetworks, DEFAULT_IPV4, NET_NAME } from "../lib/net.js";
import { SettingsIcon, RefreshIcon } from "./Icons";

/** The owner's networks, found on the relays: pick one, or create one */
function Networks({ db, current, onOpen, onSettings }) {
  const [nets, setNets] = useState(null);
  const [error, setError] = useState("");
  const [creating, setCreating] = useState(false);

  const load = async () => {
    setError("");
    try {
      setNets(await listNetworks(db));
    } catch (err) {
      setError(err.message);
      setNets([]);
    }
  };
  useEffect(() => {
    load();
  }, [db]);

  return (
    <main className="page">
      <header className="topbar">
        <h1>Networks <span className="muted small">toolnet</span></h1>
        <div className="row">
          <button className="icon-btn" onClick={() => { setNets(null); load(); }} aria-label="Refresh" disabled={!nets}>
            <span className={nets ? "" : "spin"}><RefreshIcon /></span>
          </button>
          <button className="icon-btn" onClick={onSettings} aria-label="Settings">
            <SettingsIcon />
          </button>
        </div>
      </header>

      {error && <p className="error">{error}</p>}

      {!nets ? (
        <div className="center"><div className="spinner" /><p className="muted">Looking for your networks…</p></div>
      ) : (
        <>
          {nets.length === 0 ? (
            <p className="muted empty">This owner key has no networks on these relays yet.</p>
          ) : (
            <ul className="card members">
              {nets.map((n) => (
                <li key={n.name}>
                  <button className="member" onClick={() => onOpen(n.name)} disabled={n.status !== "ok"}>
                    <span className="member__text">
                      <span className="member__name">
                        {n.name}
                        {n.name === current && <span className="muted small"> · last opened</span>}
                      </span>
                      <span className="member__detail">
                        {n.status === "ok"
                          ? `${n.ipv4} · ${n.members} ${n.members === 1 ? "device" : "devices"} · epoch ${n.epoch}`
                          : `can't open with this key (epoch ${n.epoch})`}
                      </span>
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
          {creating ? (
            <CreateNetwork db={db} taken={nets.map((n) => n.name)} onCreated={onOpen} onCancel={() => setCreating(false)} />
          ) : (
            <button className="btn btn--ghost btn--wide" onClick={() => setCreating(true)}>New network</button>
          )}
        </>
      )}
    </main>
  );
}

function CreateNetwork({ db, taken, onCreated, onCancel }) {
  const [name, setName] = useState("");
  const [ipv4, setIpv4] = useState(DEFAULT_IPV4);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const create = async (e) => {
    e.preventDefault();
    const net = name.trim();
    if (!NET_NAME.test(net)) return setError("Use lowercase letters, digits and dashes");
    if (taken.includes(net)) return setError(`You already have a network called ${net}`);
    setBusy(true);
    setError("");
    try {
      await createNetwork(db, net, { ipv4: ipv4.trim() });
      onCreated(net);
    } catch (err) {
      setError(err.message);
      setBusy(false);
    }
  };

  return (
    <section className="card">
      <h2>New network</h2>
      <p className="muted small">Same as <code>toolnet owner create --net &lt;name&gt;</code>. Devices join it with that name.</p>
      <form className="stack" onSubmit={create}>
        <label className="field">
          <span className="label">Name</span>
          <input value={name} onChange={(e) => setName(e.target.value.toLowerCase())} placeholder="home" autoCapitalize="none" autoCorrect="off" spellCheck={false} required />
        </label>
        <label className="field">
          <span className="label">Tailnet range</span>
          <input value={ipv4} onChange={(e) => setIpv4(e.target.value)} required />
        </label>
        {error && <p className="error">{error}</p>}
        <button className="btn btn--primary" type="submit" disabled={busy}>{busy ? "Creating…" : "Create"}</button>
        <button className="btn btn--ghost" type="button" onClick={onCancel}>Cancel</button>
      </form>
    </section>
  );
}

export default Networks;
