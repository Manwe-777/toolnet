import { useEffect, useRef, useState } from "react";

// BarcodeDetector where the browser has it (Android Chrome), else jsQR,
// loaded only when needed (iOS Safari, desktop Firefox)
async function makeDetector() {
  if ("BarcodeDetector" in window) {
    try {
      const formats = await window.BarcodeDetector.getSupportedFormats();
      if (formats.includes("qr_code")) {
        const detector = new window.BarcodeDetector({ formats: ["qr_code"] });
        return async (video) => (await detector.detect(video))[0]?.rawValue || null;
      }
    } catch {
      // fall back to jsQR
    }
  }
  const { default: jsQR } = await import("jsqr");
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  return async (video) => {
    const scale = Math.min(1, 640 / Math.max(video.videoWidth, video.videoHeight));
    canvas.width = Math.round(video.videoWidth * scale);
    canvas.height = Math.round(video.videoHeight * scale);
    ctx.drawImage(video, 0, 0, canvas.width, canvas.height);
    const { data, width, height } = ctx.getImageData(0, 0, canvas.width, canvas.height);
    // Terminals often draw QR codes light on dark: try both
    return jsQR(data, width, height, { inversionAttempts: "attemptBoth" })?.data || null;
  };
}

function cameraError(err) {
  if (!window.isSecureContext) return "The camera needs HTTPS (or localhost). Paste the code instead.";
  if (err?.name === "NotAllowedError") return "Camera access was denied. Allow it in the browser, or paste the code.";
  if (err?.name === "NotFoundError") return "No camera found. Paste the code instead.";
  return `Can't open the camera: ${err?.message || err}`;
}

/**
 * The camera, until it sees a QR code
 * @param onResult called with the code's text; return false to keep scanning
 */
function Scanner({ onResult }) {
  const video = useRef(null);
  const [error, setError] = useState("");
  const result = useRef(onResult);
  result.current = onResult;

  useEffect(() => {
    let stream;
    let timer;
    let stopped = false;

    (async () => {
      if (!navigator.mediaDevices?.getUserMedia) throw new Error("no camera API");
      stream = await navigator.mediaDevices.getUserMedia({
        video: { facingMode: "environment" },
        audio: false,
      });
      if (stopped) return;
      video.current.srcObject = stream;
      await video.current.play();
      const detect = await makeDetector();
      const tick = async () => {
        if (stopped) return;
        try {
          const v = video.current;
          const text = v && v.readyState >= 2 ? await detect(v) : null;
          if (text && result.current(text) !== false) return;
        } catch {
          // a bad frame, keep going
        }
        timer = setTimeout(tick, 200);
      };
      tick();
    })().catch((err) => !stopped && setError(cameraError(err)));

    return () => {
      stopped = true;
      clearTimeout(timer);
      stream?.getTracks().forEach((t) => t.stop());
    };
  }, []);

  if (error) return <p className="error">{error}</p>;
  return (
    <div className="scanner">
      <video ref={video} playsInline muted />
      <div className="scanner__frame" aria-hidden="true" />
    </div>
  );
}

export default Scanner;
