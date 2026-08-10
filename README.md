# homesign

Self-hosted sideloading pro iPad/iPhone bez závislosti na AltServeru. Docker server s webovým UI podepisuje a instaluje IPA po síti a **sám** obnovuje 7denní profily — refresh neiniciuje iOS appka (tam ho systém škrtí), ale server běžící 24/7.

## Komponenty

| Adresář | Co to je |
|---|---|
| `server/` | Rust server (axum) — web UI, REST API, podepisování, instalace přes RemoteXPC tunel, cron refresh |
| `app/` | iOS/iPadOS store appka (SwiftUI) — katalog nahraných IPA, instalace tapnutím |
| `cli/` | Pomocný nástroj pro Mac/Linux — jednorázové USB párování iPadu a upload pairing filu na server |
| `docker/` | Dockerfile + docker-compose (server + omnisette sidecar) |
| `docs/` | Architektura a poznámky |

## Provozní model

- Free Apple ID: 7denní profily, 3 sloty na appky (store + 2), 10 App ID týdně.
- Server běží doma (NAS/Pi/PC), iPad je přes noc na stejné Wi-Fi → refresh běží automaticky každých ~6 dní.
- Cíl: iPadOS 17+ (RemoteXPC/CoreDevice tunel přes `idevice`).

Detaily v [docs/architecture.md](docs/architecture.md), postup zprovoznění v [docs/setup.md](docs/setup.md).

## Stav

Ověřeno (kompiluje se, boot + smoke testy prošly):

- ✅ **Server** — axum web UI + REST API, SQLite (sqlx) persistence, upload IPA s parsováním `Info.plist`, fronta úloh, šifrování citlivých polí (AES-256-GCM, testy). Boot, migrace, API i UI ověřeny; kompiluje na Linuxu (Docker).
- ✅ **CLI** — `pair` (USB párování přes idevice, zapnutí Wi-Fi connections, upload pairing filu) + `list`. Kompiluje; reálné párování testovatelné s iPadem.
- ✅ **Instalace přes RemoteXPC (iOS 17+)** — CoreDeviceProxy → userspace software tunel (bez TUN/NET_ADMIN) → RSD → `installation_proxy` (upgrade zachová data). Kód hotový, ověření vyžaduje iPad.
- ✅ **Refresh scheduler** — hodinová kontrola expirace, přepodepsání+přeinstalace ze serveru, retry když je iPad mimo síť.
- ✅ **iOS/iPadOS store appka** — SwiftUI (katalog, instalace, expirace, živý stav úloh). Typecheck proti iOS 17 SDK prošel.
- ✅ **Apple ID auth** — GrandSlam/GSA + 2FA přes `icloud_auth` a anisette sidecar. Kód hotový, testovatelný s reálným účtem.
- ✅ **Docker** — multi-stage image + compose se serverem a omnisette sidecarem.

Zbývá (M2, vyžaduje živý účet/zařízení k dotažení):

- ⏳ **Resign** — přepis bundle id + podpis pod vlastním App ID. Teď funguje **passthrough** už podepsaných IPA (M1 tím jede end-to-end); plný resign se dolaďuje.
- ⏳ **Developer Services klient** (`apple/devportal.rs`) — registrace zařízení, cert, App ID, profil. Struktura a endpointy připravené, čeká na test proti živému účtu.

> Pozn.: knihovna `idevice` má vlastní [AI policy](https://github.com/jkcoxson/idevice/blob/master/AI.md) — případné příspěvky do ní vyžadují, aby autor kódu rozuměl a ručně psal PR popisy.
