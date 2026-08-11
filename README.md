# 🌲 Evergreen

Self-hosted sideloading for iPad/iPhone that **keeps apps alive**. Unlike AltStore/SideStore — where apps die after 7 days because the on-device refresh is throttled by iOS — Evergreen re-signs and reinstalls from a server that **you** control, so nothing has to run on the device.

The key idea: the refresh is **initiated by the server**, not by an app on the iPad. iOS can't throttle what it doesn't schedule. The iPad only needs to be paired and reachable.

## Components

| Directory | What it is |
|---|---|
| `macapp/` | Native macOS app (SwiftUI) — the UI. Bundles and launches the server, so it's one app from your side. |
| `server/` | Rust engine (axum) — REST API, Apple ID auth (GSA + anisette), Developer Services (cert/App ID/profile), IPA re-signing, transfer + install to the device. |
| `third_party/apple-private-apis` | Vendored `omnisette`/`icloud_auth` with a small patch so the server builds natively on macOS. |
| `docs/` | Architecture and setup notes. |

Why the engine is in Rust: all the heavy iOS/Apple protocol libraries live there (`idevice` for usbmux/lockdown/AFC/installation, `icloud_auth`+`omnisette` for GrandSlam auth + anisette). Reimplementing them in Swift would be months of work for no benefit. The macOS app is the native UI; the Rust server is the engine.

## How it works

1. **Pair** the iPad once over USB (done from the app — no CLI).
2. **Sign in** with a free Apple ID.
3. **Upload an IPA** and install it. The server registers the device, issues a development certificate (from your CSR), creates an App ID, downloads a provisioning profile, rewrites the bundle id to a unique one under your team, and signs the app (every nested framework individually) with your real Apple Development certificate.
4. The server keeps a copy of the cert + profile, so subsequent re-signs are **offline** (no Apple round-trip) until the profile expires.
5. A **refresh scheduler** re-signs and reinstalls before the 7-day profile expires (upgrade preserves app data). This is logged like any other job.

## Requirements

- **macOS** (Ventura or newer).
- **iPad/iPhone with Developer Mode ON** (iOS 16+): Settings → Privacy & Security → Developer Mode → on, then restart the device. Required to install development-signed apps.
- **Xcode** (or its `devicectl`, from the Command Line Tools) is required **only for wireless (Wi-Fi) install** — Evergreen hands the transfer to Apple's `devicectl` over the CoreDevice tunnel. **USB install needs no Xcode.** Without it, the app shows a warning and you install over a cable.
- A free **Apple ID** (used to sign; each user signs in with their own).

## Install

Evergreen is **not notarized** (no paid Apple Developer account), so macOS Gatekeeper warns on first launch — that's expected.

**Homebrew:**

```sh
brew tap tedyno/evergreen
brew trust tedyno/evergreen
brew install --cask evergreen
```

The middle step isn't a formality: since version 6, Homebrew refuses to load a cask from a third-party tap until you say you trust it. The cask lives in its own tap ([`tedyno/homebrew-evergreen`](https://github.com/tedyno/homebrew-evergreen)); Evergreen can't live in the official `homebrew/cask` tap, which from 2026-09-01 accepts only notarized apps.

**Manual:** download `Evergreen-<version>.zip` from [Releases](https://github.com/tedyno/evergreen/releases), unzip, drag `Evergreen.app` into `/Applications`.

Either way, the **first launch** is blocked by Gatekeeper. Do one of:
- Right-click `Evergreen.app` → **Open** → **Open** (only needed once), or
- `xattr -dr com.apple.quarantine /Applications/Evergreen.app`

The app runs the engine as a background LaunchAgent (`com.evergreen.server`), so re-signing keeps happening even when the app is closed. `brew uninstall --cask` (add `--zap` to also wipe data) removes the agent and app cleanly.

## Free Apple ID limits

7-day provisioning profiles, 3 active App IDs on a device, 10 new App IDs per week. Evergreen reuses the certificate and only contacts Apple for a fresh profile (~every 6 days).

## Status

Works, verified end-to-end on a real iPad (iOS 26) over USB:

- ✅ Native macOS app that starts/stops the embedded server; Apple ID login survives restarts (encrypted session on disk).
- ✅ USB pairing and automatic Wi-Fi IP detection, both from the app.
- ✅ Full re-sign pipeline: GSA auth (incl. the encrypted Xcode token), Developer Services (device / cert / App ID / profile), bundle-id rewrite, per-framework `codesign` + local verification. Offline re-sign from stored cert+profile (also sidesteps Apple's `-22411` rate limit).
- ✅ Install over **USB** (direct AFC via usbmux — fast) or **Wi-Fi** (Apple's `devicectl` / CoreDevice tunnel — needs Xcode). The app picks USB when a cable is present, otherwise Wi-Fi.
- ✅ Resign log (Jobs) with time / status / MB progress / errors, cancellable.

> Note: the `idevice` library has its own [AI policy](https://github.com/jkcoxson/idevice/blob/master/AI.md) — contributions to it require the code author to understand the code and write PR descriptions by hand.
