import { isExitRoute } from "../lib/parse.js";
import { ago, isRecent } from "../lib/time";
import { SettingsIcon, RefreshIcon } from "./Icons";

function NetworkView({ net, view, loading, flash, onRefresh, onAdd, onMember, onSettings, onNetworks }) {
  return (
    <main className="page">
      <header className="topbar">
        <button className="btn btn--ghost btn--small" onClick={onNetworks} aria-label="All networks">← Networks</button>
        <h1>{net}</h1>
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
        <section className="card">
          <h2>No network "{net}"</h2>
          <p className="muted">This owner key has no network called "{net}" on these relays (anymore).</p>
          <button className="btn btn--small" onClick={onNetworks}>Your networks</button>
        </section>
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

export default NetworkView;
