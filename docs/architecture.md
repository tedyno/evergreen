# Architecture

## Flow overview

```
┌─────────────┐ upload IPA, 2FA, management ┌──────────────────────────────┐
│   browser   │ ──────────────────────────► │  homesign server (Docker)    │
└─────────────┘          web UI             │                              │
                                            │  axum web UI + REST API      │
┌─────────────┐   catalog, "install"        │  SQLite (state, queues)      │
│  store app  │ ──────────────────────────► │  signer (zsign/rcodesign)    │
│   (iPad)    │ ◄────────────────────────── │  installer (idevice tunnel)  │
└─────────────┘   installation over Wi-Fi   │  scheduler (refresh ~6 days) │
                                            └──────────────┬───────────────┘
┌─────────────┐  one-time USB pairing                      │
│  cli (Mac/  │ ─────────────────────────► pairing file    │ anisette
│   Linux)    │                                            ▼
└─────────────┘                             ┌──────────────────────────────┐
                                            │  omnisette sidecar (Docker)  │
                                            └──────────────────────────────┘
```

## Key decisions

1. **The refresh is initiated by the server, not the iOS app.** A known AltStore/SideStore problem: iOS throttles Background App Refresh, so the on-device refresh runs unreliably and apps die after 7 days. The server has a cron, runs all the time, and the iPad only needs to be on a reachable network. The store app doesn't have to do anything in the background at all.
2. **Rust on the server** — the entire heavy stack exists in Rust with compatible licenses:
   - [`jkcoxson/idevice`](https://github.com/jkcoxson/idevice) (MIT) — lockdown, AFC, installation_proxy, **RemoteXPC tunnel for iOS 17+**
   - [`SideStore/apple-private-apis`](https://github.com/SideStore/apple-private-apis) (MPL-2.0) — GrandSlam/SRP auth to the Apple ID, Developer Services API
   - signing: `zsign` (MIT, C++) or `rcodesign` from apple-platform-rs — to be decided in M2
3. **Anisette as a sidecar** (`SideStore/omnisette-server`) — our own code never touches it, just HTTP. No license contamination, and when Apple breaks it, only the image changes.
4. **No code from the AltStore/SideStore apps** (AGPL) — only behavioral inspiration.

## Flows

### Bootstrap (one-time, USB)
1. Pair the iPad over USB from the app. This creates the classic lockdown pairing file, enables Wi-Fi connections, and additionally creates a RemotePairing (Ed25519) record — the second pairing iOS 17+ needs to open the RemoteXPC tunnel over Wi-Fi.
2. Apple ID sign-in (SRP + 2FA prompt); the server obtains the certificate + team.

### Installing an app
1. The IPA is uploaded from the app.
2. Server: unpacks → rewrites the bundle ID to its own App ID → generates/downloads a provisioning profile → signs (including nested frameworks and extensions) → recomputes CodeResources.
3. The server installs on the iPad, choosing the transport:
   - **USB** (preferred): direct AFC upload into PublicStaging + `installation_proxy`, over usbmux — bypasses the userspace tunnel, so bulk transfer is fast.
   - **Wi-Fi** (iOS 17+): RemotePairing verify → TLS-PSK tunnel → userspace TCP stack (jktcp) → RSD → AFC + `installation_proxy` over the tunnel.

### Refresh (automatic)
- The scheduler tracks the profile expiration for each installed app.
- Around day 6: the server re-signs and reinstalls (upgrade preserves the app's data).
- iPad unreachable → retry with backoff + notification to the store app / web UI.
- The store app itself is refreshed too.

## Free account limits

- 3 active apps per device (1 consumed by the store) — the server tracks the slots.
- 10 new App IDs / 7 days — App IDs are recycled (bundle → App ID mapping in the DB).
- 7-day profiles — see refresh.
- The 2FA session expires (~a month, sometimes sooner): the server sends a notification, the code is entered in the web UI.

## Security

- Apple ID credentials + session tokens live only on the server (encrypted at rest, the key kept outside the DB).
- Web UI behind authentication; the server is meant only for the home network / behind Tailscale, not for the public internet.

## Milestones

- **M1 — tunnel and installation:** cli pairing, the server can install an *already-signed* IPA over the network via idevice. Verification: manual install onto the iPad.
- **M2 — signing:** Apple ID auth (anisette sidecar), device registration, App ID, profile, resign IPA. Verification: the server signs and installs an arbitrary IPA.
- **M3 — web UI + persistence:** upload, list of apps/devices, 2FA flow, SQLite.
- **M4 — refresh scheduler:** automatic re-signing, retry, tracking of slots and App ID limits.
- **M5 — store app:** SwiftUI catalog, install with a tap, expiration status, notifications.
- **M6 — packaging:** multi-arch Docker image (amd64/arm64), compose, setup documentation.
