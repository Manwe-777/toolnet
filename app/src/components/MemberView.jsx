import { useState } from "react";
import { npubEncode } from "tool-db";
import { removeDevice, setRoutes } from "../lib/net.js";
import { EXIT_ROUTE, dnsName, isExitRoute, parseCidr } from "../lib/parse.js";
import { ago, isRecent } from "../lib/time";
import CopyField from "./CopyField";
import { keyInputProps } from "./Setup";

const routeLabel = (r) => (isExitRoute(r) ? `${r} (exit node)` : r);
const sameList = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);

function MemberView({ db, net, view, device, onBack, onChanged, onRemoved }) {
  const member = view.config.members[device];
  const node = view.nodes[device];

  if (!member) {
    return (
      <main className="page">
        <header className="topbar">
          <button className="btn btn--ghost btn--small" onClick={onBack}>← Back</button>
          <h1>Not a member</h1>
          <span />
        </header>
        <p className="muted">This device is no longer in the network.</p>
      </main>
    );
  }

  return (
    <main className="page">
      <header className="topbar">
        <button className="btn btn--ghost btn--small" onClick={onBack}>← Back</button>
        <h1>{member.name}</h1>
        <span />
      </header>

      <section className="card">
        <dl className="facts">
          <dt>Tailnet IP</dt>
          <dd>{member.ip}</dd>
          <dt>DNS name</dt>
          <dd><code>{dnsName(view.config, member)}</code></dd>
          <dt>Added</dt>
          <dd>{ago(member.addedAt)}</dd>
        </dl>
        <CopyField value={npubEncode(device)} />
      </section>

      <section className="card">
        <h2>
          <span className={`dot ${node && isRecent(node.updatedAt) ? "dot--up" : ""}`} /> Last report
        </h2>
        {node ? (
          <dl className="facts">
            <dt>Updated</dt>
            <dd>{ago(node.updatedAt)}</dd>
            <dt>Hostname</dt>
            <dd>{node.hostname}</dd>
            <dt>Version</dt>
            <dd>{node.version}</dd>
            <dt>Connect via</dt>
            <dd>
              {node.addrs?.length ? `${node.addrs.length} direct address${node.addrs.length === 1 ? "" : "es"}` : "no direct addresses"}
              {node.relay ? " + relay" : ""}
            </dd>
            {node.addrs?.length > 0 && (
              <>
                <dt>Addresses</dt>
                <dd className="mono">{node.addrs.map((a) => <div key={a}>{a}</div>)}</dd>
              </>
            )}
            {node.relay && (
              <>
                <dt>Relay</dt>
                <dd className="mono">{node.relay}</dd>
              </>
            )}
          </dl>
        ) : (
          <p className="muted">
            Nothing yet at this epoch. It appears once the device runs <code>toolnet up</code>.
          </p>
        )}
        <p className="hint">
          Devices report again when their addresses change, so this is when it was last heard from,
          not whether it's up right now.
        </p>
      </section>

      <Routes db={db} net={net} device={device} member={member} node={node} onChanged={onChanged} />
      <Remove db={db} net={net} device={device} member={member} onRemoved={onRemoved} />
    </main>
  );
}

function Routes({ db, net, device, member, node, onChanged }) {
  const saved = member.routes || [];
  const advertised = node?.routes || [];
  const [routes, setList] = useState(saved);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const add = (cidr) => {
    setError("");
    try {
      const r = parseCidr(cidr);
      if (!routes.includes(r)) setList([...routes, r]);
      setInput("");
    } catch (err) {
      setError(err.message);
    }
  };

  const save = async () => {
    setBusy(true);
    setError("");
    try {
      await setRoutes(db, net, device, routes);
      await onChanged(`Routes for ${member.name} saved`);
    } catch (err) {
      setError(err.message);
    }
    setBusy(false);
  };

  const offered = advertised.filter((r) => !routes.includes(r));

  return (
    <section className="card">
      <h2>Routes</h2>
      <p className="muted small">
        Prefixes this device may route for the network: its LAN, or <code>{EXIT_ROUTE}</code> to be
        an exit node. They only work if the device also advertises them.
      </p>

      {routes.length === 0 ? (
        <p className="muted">None allowed.</p>
      ) : (
        <ul className="list">
          {routes.map((r) => (
            <li key={r} className="list__item">
              <span>
                <code>{r}</code> {isExitRoute(r) && <span className="tag">exit node</span>}
                {!advertised.includes(r) && <span className="muted small"> · not advertised</span>}
              </span>
              <button className="btn btn--small btn--ghost" onClick={() => setList(routes.filter((x) => x !== r))}>
                Remove
              </button>
            </li>
          ))}
        </ul>
      )}

      {offered.length > 0 && (
        <div className="offered">
          <p className="muted small">Advertised by the device:</p>
          {offered.map((r) => (
            <button key={r} className="chip" onClick={() => add(r)}>+ {routeLabel(r)}</button>
          ))}
        </div>
      )}

      <form
        className="row"
        onSubmit={(e) => {
          e.preventDefault();
          add(input);
        }}
      >
        <input value={input} onChange={(e) => setInput(e.target.value)} placeholder="192.168.1.0/24" inputMode="decimal" {...keyInputProps} />
        <button className="btn btn--small" type="submit">Add</button>
      </form>
      {!routes.includes(EXIT_ROUTE) && (
        <button className="btn btn--small btn--ghost" onClick={() => add(EXIT_ROUTE)}>+ Allow as exit node</button>
      )}
      {error && <p className="error">{error}</p>}
      {!sameList(routes, saved) && (
        <div className="row actions">
          <button className="btn btn--primary" onClick={save} disabled={busy}>{busy ? "Saving…" : "Save routes"}</button>
          <button className="btn btn--ghost" onClick={() => setList(saved)} disabled={busy}>Undo</button>
        </div>
      )}
    </section>
  );
}

function Remove({ db, net, device, member, onRemoved }) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const remove = async () => {
    setBusy(true);
    setError("");
    try {
      const done = await removeDevice(db, net, device);
      onRemoved(`Removed ${done.name} (${done.ip}). Group key rotated to epoch ${done.epoch}.`);
    } catch (err) {
      setError(err.message);
      setBusy(false);
    }
  };

  return (
    <section className="card">
      {confirming ? (
        <div className="stack">
          <p>
            Remove <b>{member.name}</b>? It loses access right away: the network gets a new group key
            that every other device receives, and {member.name} doesn't.
          </p>
          {error && <p className="error">{error}</p>}
          <button className="btn btn--danger-solid" onClick={remove} disabled={busy}>
            {busy ? "Removing…" : `Remove ${member.name}`}
          </button>
          <button className="btn btn--ghost" onClick={() => setConfirming(false)} disabled={busy}>Cancel</button>
        </div>
      ) : (
        <button className="btn btn--danger btn--wide" onClick={() => setConfirming(true)}>Remove from network</button>
      )}
    </section>
  );
}

export default MemberView;
