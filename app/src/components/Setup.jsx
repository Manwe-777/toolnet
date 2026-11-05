import { useState } from "react";
import { saveSecretKey } from "../lib/session";

// Keys are case sensitive to keyboards: no capitals, corrections or suggestions
export const keyInputProps = {
  autoCapitalize: "none",
  autoCorrect: "off",
  autoComplete: "off",
  spellCheck: false,
};

// The public copy: its storage (where the key is kept) is shared with every
// other site on the same github.io origin. The daemon serves a private copy
const PUBLIC_COPY = location.hostname.endsWith(".github.io");

function Setup({ onDone }) {
  const [key, setKey] = useState("");
  const [error, setError] = useState("");

  const submit = (e) => {
    e.preventDefault();
    setError("");
    try {
      onDone(saveSecretKey(key));
    } catch {
      setError("That's not a valid secret key (nsec1… or 64 hex characters)");
    }
  };

  return (
    <main className="center setup">
      <img src="./icon.svg" alt="" className="setup__logo" />
      <h1>toolnet owner</h1>
      <p className="muted">Approve and remove the devices of your toolnet network.</p>
      {PUBLIC_COPY && (
        <p className="notice notice--bad">
          This is the online copy: the key would be kept in storage that other github.io sites
          share. On a machine running toolnet, open <code>http://127.0.0.1:7477</code> instead
          (or the tray's “Open the owner app”); from another member, this node's tailnet address
          on port 7477.
        </p>
      )}
      <form className="stack" onSubmit={submit}>
        <input
          type="password"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          placeholder="nsec1… or hex"
          aria-label="Owner secret key"
          autoFocus
          required
          {...keyInputProps}
        />
        {error && <p className="error">{error}</p>}
        <button className="btn btn--primary" type="submit">Sign in</button>
        <p className="hint">
          Your owner key: the contents of <code>owner.key</code> from{" "}
          <code>toolnet owner create</code> (in <code>~/.local/share/toolnet</code>). It stays in
          this browser.
        </p>
      </form>
    </main>
  );
}

export default Setup;
