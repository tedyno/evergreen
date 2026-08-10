# Setup

From zero to an app that keeps itself alive on your iPad. Everything runs natively on macOS — no Docker, no CLI.

## 0. Prerequisites

- A Mac (Apple Silicon or Intel) with Xcode command-line tools.
- `xcodegen` (`brew install xcodegen`) to generate the Xcode project.
- An iPad on iOS/iPadOS 17+ with **Developer Mode** enabled (Settings → Privacy & Security → Developer Mode).
- A free Apple ID.

## 1. Build & run the app

```bash
cargo build --release -p homesign-server      # build the engine
cd macapp && xcodegen generate                 # generate the Xcode project
xcodebuild -project Evergreen.xcodeproj -scheme Evergreen -configuration Debug \
  -derivedDataPath build build                 # build the app (embeds the server)
open build/Build/Products/Debug/Evergreen.app
```

The app launches the embedded server on `127.0.0.1:8080` and stores its data in `~/Library/Application Support/homesign`.

## 2. Pair the iPad (once, over USB)

Connect the iPad with a cable and unlock it, then in the app: **Devices → Pair iPad**. Confirm **Trust** on the iPad. The app enables Wi-Fi connections and automatically detects the iPad's IP.

## 3. Sign in with your Apple ID

**Account** tab → Apple ID + password → enter the 2FA code from a trusted device. The password is stored encrypted (AES-256-GCM) and is only used for signing. The login survives app restarts.

## 4. Upload an IPA and install

**Apps** tab → drop an `.ipa` → select the device → **Install**. Watch progress in **Jobs**. The server registers the device, issues a cert, creates an App ID, downloads a profile, rewrites the bundle id, signs the app (each nested framework), and installs it over USB.

## 5. Automatic renewal

You don't have to do anything. The server checks profile expiration hourly and re-signs + reinstalls the ones that are about to expire (upgrade preserves app data). Every re-sign shows up in **Jobs**.

> Current limitation: install/refresh works over **USB** (the iPad must be connected). Wireless install on iOS 17+ needs the RemotePairing subsystem, which isn't implemented yet.

## Troubleshooting

| Problem | Cause / fix |
|---|---|
| Install fails with `CoreDeviceProxy: peer closed connection` | Developer Mode is off, or the iPad is locked. |
| `-22411 "This action cannot be completed at this time"` | Apple rate-limited the token endpoint. Wait (don't keep retrying); the token is cached so it won't recur. |
| Pairing rejected (`UserDeniedPairing`) | Unlock the iPad and confirm the Trust dialog; reconnect the cable if it doesn't appear. |
