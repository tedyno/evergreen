-- Apple ID účet (jeden na server pro začátek). Citlivá pole šifrovaná.
CREATE TABLE IF NOT EXISTS account (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    apple_id     TEXT NOT NULL,
    password_enc TEXT NOT NULL,           -- AES-GCM
    session_enc  TEXT,                     -- serializovaný login stav / adsid token
    team_id      TEXT,
    updated_at   TEXT NOT NULL
);

-- Spárovaná zařízení.
CREATE TABLE IF NOT EXISTS device (
    udid          TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    address       TEXT,                    -- poslední známá IP v síti (Wi-Fi)
    pairing_path  TEXT NOT NULL,           -- cesta k pairing file v data_dir
    model         TEXT,
    ios_version   TEXT,
    created_at    TEXT NOT NULL,
    last_seen     TEXT
);

-- Nahrané IPA (katalog).
CREATE TABLE IF NOT EXISTS ipa (
    id            TEXT PRIMARY KEY,        -- uuid
    filename      TEXT NOT NULL,
    bundle_id     TEXT NOT NULL,           -- původní bundle id z Info.plist
    name          TEXT NOT NULL,
    version       TEXT,
    size_bytes    INTEGER NOT NULL,
    path          TEXT NOT NULL,           -- cesta v data_dir/ipa
    icon_path     TEXT,
    created_at    TEXT NOT NULL
);

-- Instalace: konkrétní IPA na konkrétním zařízení, s podepsaným bundle id a expirací.
CREATE TABLE IF NOT EXISTS installation (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    device_udid       TEXT NOT NULL REFERENCES device(udid) ON DELETE CASCADE,
    ipa_id            TEXT NOT NULL REFERENCES ipa(id) ON DELETE CASCADE,
    signed_bundle_id  TEXT NOT NULL,       -- přepsané bundle id (App ID)
    app_id_ext        TEXT,                -- identifier App ID na Apple portálu
    profile_expires   TEXT,                -- ISO8601; refresh se řídí tímto
    last_installed    TEXT,
    status            TEXT NOT NULL DEFAULT 'pending', -- pending|installed|expired|error
    error             TEXT,
    UNIQUE (device_udid, ipa_id)
);

-- Fronta úloh (install/refresh) — přežije restart serveru.
CREATE TABLE IF NOT EXISTS job (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    kind         TEXT NOT NULL,           -- install|refresh|pair_test
    device_udid  TEXT,
    ipa_id       TEXT,
    status       TEXT NOT NULL DEFAULT 'queued', -- queued|running|done|error
    progress     INTEGER NOT NULL DEFAULT 0,
    message      TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
