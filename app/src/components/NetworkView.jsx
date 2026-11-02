import { useState } from "react";
import { createNetwork, DEFAULT_IPV4 } from "../lib/net.js";
import { isExitRoute } from "../lib/parse.js";
import { ago, isRecent } from "../lib/time";
import { SettingsIcon, RefreshIcon } from "./Icons";

function NetworkView({ db, net, view, loading, flash, onRefresh, onAdd, onMember, onSettings }) {
  return (
    <main className="page">
      <header className="topbar">
        <h1>
          {net} <span className="muted small">toolnet</span>
        </h1>
        <div className="row">
          <button className="icon-btn" onClick={onRefresh} aria-label="Refresh" disabled={loading}>
            <span className={loading ? "spin" : ""}><RefreshIcon /></span>
          </button>
          <button className="icon-btn" onClick={onSettings} aria-label="Settings">
            <SettingsIcon />
          </button>
        </div>
      </header>

      {flash && <p className="notice">{flash}</p>}

      {!view ? (
        <div className="center"><div className="spinner" /></div>
      ) : view.status === "none" ? (
        <CreateNetwork db={db} net={net} onCreated={onRefresh} onSettings={onSettings} />
      ) : view.status === "locked" ? (
        <section className="card">
          <h2>Can't open "{net}"</h2>
          <p className="muted">
            The network exists (epoch {view.epoch}) but this key has no group key for it. Is this
            the right owner key?
          </p>
          <button className="btn btn--small" onClick={onSettings}>Settings</button>
        </section>
      ) : (
        <Members view={view} onAdd={onAdd} onMember={onMember} />
      )}
    </main>
  );
}

function Members({ view, onAdd, onMember }) {
  const members = Object.entries(view.config.members).sort(([, a], [, b]) => ipOrder(a.ip) - ipOrder(b.ip));
  return (
    <>
      <p className="muted small summary">
        {view.config.ipv4} · epoch {view.epoch} · {members.length} {members.length === 1 ? "device" : "devices"}
      </p>
      <button className="btn btn--primary btn--wide" onClick={onAdd}>Add a device</button>

      {members.length === 0 ? (
        <p className="muted empty">No devices yet.</p>
      ) : (
        <ul className="card members">
          {members.map(([key, member]) => {
            const node = view.nodes[key];
            return (
              <li key={key}>
                <button className="member" onClick={() => onMember(key)}>
                  <span className={`dot ${node && isRecent(node.updatedAt) ? "dot--up" : ""}`} />
                  <span className="member__text">
                    <span className="member__name">
                      {member.name}
                      {(member.routes || []).some(isExitRoute) && <span className="tag">exit node</span>}
                      {(member.routes || []).some((r) => !isExitRoute(r)) && <span className="tag">subnets</span>}
                    </span>
                    <span className="member__detail">
                      {member.ip}
                      {node ? ` · ${node.hostname} · ${ago(node.updatedAt)}` : " · not seen yet"}
                    </span>
                  </span>
                  <span className="muted">›</span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </>
  );
}

const ipOrder = (ip) => ip.split(".").reduce((n, p) => n * 256 + Number(p), 0);

function CreateNetwork({ db, net, onCreated, onSettings }) {
  const [ipv4, setIpv4] = useState(DEFAULT_IPV4);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const create = async (e) => {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      await createNetwork(db, net, { ipv4: ipv4.trim() });
      onCreated();
    } catch (err) {
      setError(err.message);
    }
    setBusy(false);
  };

  return (
    <section className="card">
      <h2>No network "{net}" yet</h2>
      <p className="muted">
        This owner key has no network called "{net}" on these relays. If you made it with{" "}
        <code>toolnet owner create</code>, check the network name and relays in settings. Or create
        it here:
      </p>
      <form className="stack" onSubmit={create}>
        <label className="field">
          <span className="label">Tailnet range</span>
          <input value={ipv4} onChange={(e) => setIpv4(e.target.value)} required />
        </label>
        {error && <p className="error">{error}</p>}
        <button className="btn btn--primary" type="submit" disabled={busy}>
          {busy ? "Creating…" : `Create ${net}`}
        </button>
        <button className="btn btn--ghost" type="button" onClick={onSettings}>Settings</button>
      </form>
    </section>
  );
}

export default NetworkView;
