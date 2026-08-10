import Foundation

// DTOs mirroring the server (`server/src/models.rs` + `web/api.rs`).

struct ServerStatus: Decodable {
    let name: String
    let version: String
    let ok: Bool
}

struct Account: Decodable {
    let linked: Bool
    let appleId: String?
    let teamId: String?
    let authState: String   // "logged_out" | "needs_2fa" | "logged_in"

    enum CodingKeys: String, CodingKey {
        case linked
        case appleId = "apple_id"
        case teamId = "team_id"
        case authState = "auth_state"
    }
}

struct Device: Decodable, Identifiable {
    let udid: String
    let name: String
    let address: String?
    let model: String?
    let iosVersion: String?
    let createdAt: String
    let lastSeen: String?

    var id: String { udid }

    enum CodingKeys: String, CodingKey {
        case udid, name, address, model
        case iosVersion = "ios_version"
        case createdAt = "created_at"
        case lastSeen = "last_seen"
    }
}

struct Ipa: Decodable, Identifiable {
    let id: String
    let filename: String
    let bundleId: String
    let name: String
    let version: String?
    let sizeBytes: Int64
    let iconPath: String?
    let createdAt: String

    enum CodingKeys: String, CodingKey {
        case id, filename, name, version
        case bundleId = "bundle_id"
        case sizeBytes = "size_bytes"
        case iconPath = "icon_path"
        case createdAt = "created_at"
    }
}

struct HSJob: Decodable, Identifiable {
    let id: Int64
    let kind: String
    let deviceUdid: String?
    let ipaId: String?
    let status: String       // "queued" | "running" | "done" | "error" | ...
    let progress: Int64
    let message: String?
    let createdAt: String
    let updatedAt: String

    enum CodingKeys: String, CodingKey {
        case id, kind, status, progress, message
        case deviceUdid = "device_udid"
        case ipaId = "ipa_id"
        case createdAt = "created_at"
        case updatedAt = "updated_at"
    }

    var isActive: Bool { status == "running" || status == "queued" }

    /// Creation time as a Date.
    var createdDate: Date? { HSJob.parseDate(createdAt) }
    var updatedDate: Date? { HSJob.parseDate(updatedAt) }

    static func parseDate(_ s: String) -> Date? {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return f.date(from: s) ?? {
            let g = ISO8601DateFormatter()
            g.formatOptions = [.withInternetDateTime]
            return g.date(from: s)
        }()
    }
}

struct Installation: Decodable, Identifiable {
    let id: Int64
    let deviceUdid: String
    let ipaId: String
    let signedBundleId: String
    let appIdExt: String?
    let profileExpires: String?
    let lastInstalled: String?
    let status: String
    let error: String?

    enum CodingKeys: String, CodingKey {
        case id, status, error
        case deviceUdid = "device_udid"
        case ipaId = "ipa_id"
        case signedBundleId = "signed_bundle_id"
        case appIdExt = "app_id_ext"
        case profileExpires = "profile_expires"
        case lastInstalled = "last_installed"
    }

    /// Number of whole days until the profile expires (nil = unknown).
    var daysUntilExpiry: Int? {
        guard let s = profileExpires else { return nil }
        let fmt = ISO8601DateFormatter()
        fmt.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let date = fmt.date(from: s) ?? ISO8601DateFormatter().date(from: s)
        guard let date else { return nil }
        return Calendar.current.dateComponents([.day], from: Date(), to: date).day
    }

    var isActive: Bool {
        status != "removed" && status != "error"
    }

    var lastInstalledDate: Date? { lastInstalled.flatMap { HSJob.parseDate($0) } }
    var expiryDate: Date? { profileExpires.flatMap { HSJob.parseDate($0) } }
}

struct OkResponse: Decodable {
    let ok: Bool
}

struct AppIdEntry: Decodable, Identifiable {
    let appIdId: String
    let identifier: String
    let name: String
    let expiration: String?

    var id: String { appIdId }

    enum CodingKeys: String, CodingKey {
        case appIdId = "app_id_id"
        case identifier, name, expiration
    }
}

/// The actual App ID state on the Apple account (from Developer Services).
struct AppIdInfo: Decodable {
    let teamId: String
    let count: Int
    let max: Int
    let appIds: [AppIdEntry]

    enum CodingKeys: String, CodingKey {
        case teamId = "team_id"
        case count, max
        case appIds = "app_ids"
    }
}

/// Response to login / 2FA.
struct AuthOutcome: Decodable {
    let state: String        // "logged_in" | "needs_2fa"
    let teamId: String?

    enum CodingKeys: String, CodingKey {
        case state
        case teamId = "team_id"
    }
}
