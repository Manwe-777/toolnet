# toolnet owner app

A small PWA for the network owner, made for a phone: sign in with the owner
key, see the members, approve devices by scanning a QR code, edit their
routes, remove them. It writes exactly what `toolnet owner approve / routes /
remove` write (see `../daemon/README.md`), so the CLI and the app can be used
side by side.

```bash
npm install
npm run dev        # http://localhost:5173, and on your LAN
npm run build      # static files in dist/, serve them from anywhere (relative paths)
```

## Using it

- **Sign in** with the owner key: `nsec1…`, or the hex in `owner.key`
  (`~/.local/share/toolnet/owner.key` on the machine where you ran
  `toolnet owner create`). It stays in this browser's localStorage.
- **Network**: `home` by default; change it, and the Nostr relays, in
  settings. Use the relays your devices use (their `config.json`).
- **Add a device**: on the device run `toolnet init --owner <npub> --net home`,
  then `toolnet id --qr`, and scan the code
  (`toolnet:join?device=npub1…&name=…`). Pasting the code, an npub or a 64-hex
  key works too. The IP defaults to the next free one, like the CLI.
- **Remove** rotates the group key (new epoch), wrapped for the owner and the
  remaining members only.
- **Routes**: allowed prefixes per member (`0.0.0.0/0` = exit node). They only
  take effect if the device also advertises them (`toolnet up
  --advertise-routes` / `--advertise-exit-node`).
- A member's "last report" is when its node info last changed: devices only
  republish when their addresses change, so it's "last heard from", not a
  live online state.

## Camera: HTTPS or localhost

Browsers only give the camera to secure contexts: serve the app over
**HTTPS**, or open it on `localhost`. Over plain `http://192.168.x.y:5173`
from a phone the scanner can't start; paste the code instead, or use
`vite --https` / a reverse proxy with a certificate. QR codes are read with
`BarcodeDetector` where the browser has it (Android Chrome) and with jsQR
otherwise (iOS Safari, Firefox), loaded only then.

## Code

```
src/lib/parse.js   pubkeys, join QR payloads, CIDRs, next free IP (match the Rust daemon)
src/lib/net.js     the owner's reads and writes in tool-db (no browser APIs)
src/lib/db.js      connects with IndexedDB storage
src/lib/session.js owner key, relays and network name in localStorage
```

`parse.js` and `net.js` run in Node too: `../tests/app-compat.sh` drives them
against the Rust daemon on a local relay (needs `npm install` here, a built
`../../tool-db` checkout and `cargo build -p toolnet`).
