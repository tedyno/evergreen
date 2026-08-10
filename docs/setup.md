# Setup

Kompletní postup od nuly k appce, která se sama obnovuje na iPadu.

## 0. Předpoklady

- Docker + docker-compose na stroji, který běží doma 24/7 (NAS / Raspberry Pi / mini PC).
- Mac nebo Linux s USB portem pro **jednorázové** spárování iPadu.
- iPad na stejné Wi-Fi jako server, iPadOS 17+.
- Apple ID (free stačí).

## 1. Spusť server

```bash
cd docker
docker compose up -d
```

Běží:
- `homesign` na `http://<server-ip>:8080` (web UI + API),
- `anisette` (omnisette) sidecar pro Apple ID auth.

Otevři `http://<server-ip>:8080` — mělo by naběhnout UI.

## 2. Spáruj iPad (jednorázově, přes USB)

Na Macu/Linuxu s připojeným a odemčeným iPadem:

```bash
cargo run -p homesign-cli -- pair \
  --server http://<server-ip>:8080 \
  --address <ip-ipadu-ve-wifi>
```

- Na iPadu potvrď **Trust / Důvěřovat**.
- CLI spáruje, zapne Wi-Fi connections a nahraje pairing file na server.
- `--address` je IP iPadu ve Wi-Fi (Nastavení → Wi-Fi → i). Bez ní ji doplň ve web UI.

> Proč USB jen jednou: iPad umí instalace přijímat po Wi-Fi, ale musí nejdřív
> vzniknout důvěryhodný pairing, což jde jen po kabelu.

## 3. Přihlas Apple ID (ve web UI)

Záložka **Účet** → Apple ID + heslo → zadej 2FA kód z důvěryhodného zařízení.
Heslo se ukládá šifrovaně (AES-256-GCM) a slouží jen k podpisu.

## 4. Nahraj IPA a nainstaluj

Záložka **Aplikace** → přetáhni `.ipa` → vyber zařízení → **Instalovat**.
Průběh sleduj v **Úlohy**.

## 5. Automatická obnova

O nic se nestaráš. Server každou hodinu kontroluje expiraci profilů a ty, kterým
zbývá méně než den, sám přepodepíše a přeinstaluje (upgrade zachová data appky).
Podmínka: iPad je v tu chvíli dosažitelný na síti. Refresh iniciuje **server**,
takže ho iOS nemá jak zaškrtit na pozadí — to je hlavní důvod, proč tohle
řešení nezhasne po 7 dnech jako AltStore/SideStore.

## 6. (Volitelně) store appka na iPadu

`app/` — SwiftUI klient. Vygeneruj projekt a nainstaluj přes homesign samotný:

```bash
brew install xcodegen
cd app && xcodegen generate
# otevři HomesignStore.xcodeproj, archivuj do .ipa, nahraj přes web UI
```

V appce nastav adresu serveru; uvidíš katalog, stav instalací a expirace.

## Řešení potíží

| Problém | Příčina / řešení |
|---|---|
| CLI: „nelze se připojit k usbmuxd" | Na macOS běží automaticky; na Linuxu doinstaluj `usbmuxd`. |
| Instalace selže na `CoreDeviceProxy` | iPad mimo síť nebo neplatný pairing → spusť `pair` znovu. |
| Auth selže hned po hesle | Anisette sidecar neběží nebo ho Apple změnil → aktualizuj image. |
| Appka zmizela po 7 dnech | iPad nebyl v okně obnovy na síti; po návratu se přeinstaluje. |

## Známá omezení (viz architecture.md)

- **Resign** (přepis bundle id + podpis pod vlastním App ID) je M2 — teď funguje
  passthrough už podepsaných IPA. Plný resign se dolaďuje proti živému účtu.
- **Developer Services** klient (registrace zařízení, cert, profil) je rozepsaný
  v `apple/devportal.rs` a je hlavním zbývajícím kusem M2.
- Free účet: max 3 appky (1 = store), profily 7 dní, 10 App ID/týden.
- **Refresh vs. passthrough:** skutečné prodloužení 7denní expirace vyžaduje při
  každé obnově *čerstvý* profil, tj. resign (M2). U passthrough už podepsaných IPA
  scheduler přeinstaluje tentýž bundle, ale expiraci free-účtu neprodlouží —
  passthrough dává smysl hlavně pro IPA z placeného účtu (roční profil), kde
  refresh stejně není potřeba. Automatická obnova free profilů se plně rozjede
  s dokončením M2.
