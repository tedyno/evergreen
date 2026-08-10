# Architektura

## Přehled toku

```
┌─────────────┐   upload IPA, 2FA, správa   ┌──────────────────────────────┐
│  prohlížeč  │ ──────────────────────────► │  homesign server (Docker)    │
└─────────────┘         web UI              │                              │
                                            │  axum web UI + REST API      │
┌─────────────┐   katalog, "nainstaluj"     │  SQLite (stav, fronty)       │
│  store app  │ ──────────────────────────► │  signer (zsign/rcodesign)    │
│  (iPad)     │ ◄────────────────────────── │  installer (idevice tunel)   │
└─────────────┘   instalace po Wi-Fi        │  scheduler (refresh ~6 dní)  │
                                            └──────────────┬───────────────┘
┌─────────────┐  jednorázové USB párování                  │
│  cli (Mac/  │ ─────────────────────────► pairing file    │ anisette
│  Linux)     │                                            ▼
└─────────────┘                             ┌──────────────────────────────┐
                                            │  omnisette sidecar (Docker)  │
                                            └──────────────────────────────┘
```

## Klíčová rozhodnutí

1. **Refresh iniciuje server, ne iOS appka.** Známý problém AltStore/SideStore: iOS škrtí Background App Refresh, takže on-device refresh nespolehlivě běží a appky po 7 dnech umírají. Server má cron, běží pořád a iPad jen musí být na dosažitelné síti. Store appka nemusí na pozadí dělat vůbec nic.
2. **Rust na serveru** — celý těžký stack existuje v Rustu s kompatibilními licencemi:
   - [`jkcoxson/idevice`](https://github.com/jkcoxson/idevice) (MIT) — lockdown, AFC, installation_proxy, **RemoteXPC tunel pro iOS 17+**
   - [`SideStore/apple-private-apis`](https://github.com/SideStore/apple-private-apis) (MPL-2.0) — GrandSlam/SRP auth k Apple ID, Developer Services API
   - podepisování: `zsign` (MIT, C++) nebo `rcodesign` z apple-platform-rs — rozhodnout v M2
3. **Anisette jako sidecar** (`SideStore/omnisette-server`) — vlastní kód se ho nedotýká, jen HTTP. Žádná licenční kontaminace, a když ho Apple rozbije, mění se jen image.
4. **Žádný kód z AltStore/SideStore appek** (AGPL) — jen inspirace chováním.

## Toky

### Bootstrap (jednorázově, Mac/Linux + USB)
1. `cli pair` — přes USB vytvoří pairing file, zapne Wi-Fi connections, nahraje pairing na server.
2. Web UI: přihlášení Apple ID (SRP + 2FA prompt), server získá certifikát + tým.
3. Server podepíše store appku a nainstaluje ji na iPad po síti.

### Instalace appky
1. IPA se nahraje přes web UI (nebo vybere ve store appce z katalogu).
2. Server: rozbalí → přepíše bundle ID na vlastní App ID → vygeneruje/stáhne provisioning profil → podepíše (včetně nested frameworků a extensions) → přepočítá CodeResources.
3. Server naváže RemoteXPC tunel na iPad (pairing file, CoreDevice), AFC upload do PublicStaging, `installation_proxy` install.

### Refresh (automaticky)
- Scheduler drží pro každou nainstalovanou appku expiraci profilu.
- ~den 6: server přepodepíše a přeinstaluje (upgrade zachovává data appky).
- iPad nedosažitelný → retry s backoffem + notifikace do store appky / web UI.
- Refreshuje se i samotná store appka.

## Omezení free účtu

- 3 aktivní appky na zařízení (1 spotřebuje store) — server hlídá sloty.
- 10 nových App ID / 7 dní — App ID se recyklují (mapování bundle → App ID v DB).
- Profily 7 dní — viz refresh.
- 2FA session vyprší (~měsíc i dřív): server pošle notifikaci, kód se zadá ve web UI.

## Bezpečnost

- Apple ID credentials + session tokeny žijí jen na serveru (šifrované at rest, klíč mimo DB).
- Web UI za autentizací; server je určen jen do domácí sítě / za Tailscale, ne na veřejný internet.

## Milníky

- **M1 — tunel a instalace:** cli párování, server umí přes idevice nainstalovat *už podepsanou* IPA po síti. Ověření: ruční install na iPad.
- **M2 — podepisování:** Apple ID auth (anisette sidecar), registrace zařízení, App ID, profil, resign IPA. Ověření: server podepíše a nainstaluje libovolnou IPA.
- **M3 — web UI + persistence:** upload, seznam appek/zařízení, 2FA flow, SQLite.
- **M4 — refresh scheduler:** automatické přepodepsání, retry, hlídání slotů a App ID limitů.
- **M5 — store appka:** SwiftUI katalog, instalace tapnutím, stav expirace, notifikace.
- **M6 — packaging:** multi-arch Docker image (amd64/arm64), compose, dokumentace setupu.
