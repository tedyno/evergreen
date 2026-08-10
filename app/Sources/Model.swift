import Foundation

// Datové typy zrcadlící REST API homesign serveru.

struct IpaItem: Identifiable, Codable, Hashable {
    let id: String
    let filename: String
    let bundle_id: String
    let name: String
    let version: String?
    let size_bytes: Int64
    let icon_path: String?
    let created_at: String
}

struct DeviceItem: Identifiable, Codable, Hashable {
    var id: String { udid }
    let udid: String
    let name: String
    let address: String?
    let model: String?
    let ios_version: String?
}

struct JobItem: Identifiable, Codable, Hashable {
    let id: Int64
    let kind: String
    let status: String
    let progress: Int64
    let message: String?
}

struct InstallationItem: Identifiable, Codable, Hashable {
    let id: Int64
    let device_udid: String
    let ipa_id: String
    let signed_bundle_id: String
    let profile_expires: String?
    let status: String
}

struct AccountStatus: Codable {
    let linked: Bool
    let apple_id: String?
    let team_id: String?
    let auth_state: String
}

struct InstallRequest: Codable {
    let device_udid: String
    let ipa_id: String
}
