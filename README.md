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

Detaily v [docs/architecture.md](docs/architecture.md).
