import { useState } from "react";

function CopyField({ value, label }) {
  const [copied, setCopied] = useState(false);

  const copy = async () => {
    await navigator.clipboard.writeText(value);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="copy-field">
      {label && <span className="copy-field__label">{label}</span>}
      <code className="copy-field__value">{value}</code>
      <button type="button" className="btn btn--small" onClick={copy}>
        {copied ? "Copied" : "Copy"}
      </button>
    </div>
  );
}

export default CopyField;
