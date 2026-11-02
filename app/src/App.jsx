import { useCallback, useEffect, useRef, useState } from "react";
import { getSecretKey, forgetSecretKey, getNet, saveNet } from "./lib/session";
import { connect } from "./lib/db";
import { loadNetwork } from "./lib/net.js";
import Setup from "./components/Setup";
import NetworkView from "./components/NetworkView";
import AddDevice from "./components/AddDevice";
import MemberView from "./components/MemberView";
import Settings from "./components/Settings";

const REFRESH_MS = 30000;

function App() {
  const [secretKey, setSecretKey] = useState(getSecretKey);
  const [net, setNet] = useState(getNet);
  const [db, setDb] = useState(null);
  const [error, setError] = useState("");
  const [view, setView] = useState(null);
  const [loading, setLoading] = useState(false);
  const [screen, setScreen] = useState({ name: "net" });
  const [flash, setFlash] = useState("");
  const loadId = useRef(0);

  useEffect(() => {
    if (!secretKey) return;
    let cancelled = false;
    let instance;
    connect(secretKey)
      .then((connected) => {
        instance = connected;
        if (cancelled) connected.close();
        else setDb(connected);
      })
      .catch((err) => setError(err.message));
    return () => {
      cancelled = true;
      instance?.close();
      setDb(null);
    };
  }, [secretKey]);

  const reload = useCallback(async () => {
    if (!db) return;
    const id = ++loadId.current;
    setLoading(true);
    try {
      const next = await loadNetwork(db, net);
      if (id === loadId.current) setView(next);
    } catch (err) {
      if (id === loadId.current) setFlash(`Could not load the network: ${err.message}`);
    }
    if (id === loadId.current) setLoading(false);
  }, [db, net]);

  // Load now, then every so often while visible
  useEffect(() => {
    if (!db) return;
    setView(null);
    reload();
    const timer = setInterval(() => document.visibilityState === "visible" && reload(), REFRESH_MS);
    const onVisible = () => document.visibilityState === "visible" && reload();
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [db, reload]);

  useEffect(() => {
    if (!flash) return;
    const timer = setTimeout(() => setFlash(""), 6000);
    return () => clearTimeout(timer);
  }, [flash]);

  const done = async (message) => {
    setFlash(message);
    setScreen({ name: "net" });
    await reload();
  };

  const signOut = () => {
    forgetSecretKey();
    setView(null);
    setSecretKey(null);
  };

  if (!secretKey) return <Setup onDone={setSecretKey} />;

  if (error) {
    return (
      <main className="center">
        <p className="error">Could not connect: {error}</p>
        <button className="btn" onClick={() => location.reload()}>Retry</button>
      </main>
    );
  }

  if (!db) {
    return (
      <main className="center">
        <div className="spinner" />
        <p className="muted">Connecting to relays…</p>
      </main>
    );
  }

  if (screen.name === "settings") {
    return (
      <Settings
        secretKey={secretKey}
        net={net}
        onNet={(name) => {
          saveNet(name);
          setNet(name);
          setScreen({ name: "net" });
        }}
        onSignOut={signOut}
        onBack={() => setScreen({ name: "net" })}
      />
    );
  }

  if (screen.name === "add" && view?.status === "ok") {
    return (
      <AddDevice
        db={db}
        net={net}
        view={view}
        onDone={(d) => done(`Approved ${d.name} at ${d.ip}`)}
        onCancel={() => setScreen({ name: "net" })}
      />
    );
  }

  if (screen.name === "member" && view?.status === "ok") {
    return (
      <MemberView
        key={screen.device}
        db={db}
        net={net}
        view={view}
        device={screen.device}
        onBack={() => setScreen({ name: "net" })}
        onChanged={async (message) => {
          setFlash(message);
          await reload();
        }}
        onRemoved={done}
      />
    );
  }

  return (
    <NetworkView
      db={db}
      net={net}
      view={view}
      loading={loading}
      flash={flash}
      onRefresh={reload}
      onAdd={() => setScreen({ name: "add" })}
      onMember={(device) => setScreen({ name: "member", device })}
      onSettings={() => setScreen({ name: "settings" })}
    />
  );
}

export default App;
