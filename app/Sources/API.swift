import Foundation

/// Tenký HTTP klient proti homesign serveru.
@MainActor
final class API: ObservableObject {
    @Published var serverURL: String {
        didSet { UserDefaults.standard.set(serverURL, forKey: "server") }
    }

    init() {
        let saved = UserDefaults.standard.string(forKey: "server")
        let def = (Bundle.main.object(forInfoDictionaryKey: "HSDefaultServer") as? String)
            ?? "http://homesign.local:8080"
        self.serverURL = saved ?? def
    }

    enum APIError: LocalizedError {
        case badURL, http(String)
        var errorDescription: String? {
            switch self {
            case .badURL: return "Neplatná adresa serveru"
            case .http(let m): return m
            }
        }
    }

    private func base() throws -> URL {
        guard let u = URL(string: serverURL) else { throw APIError.badURL }
        return u
    }

    private func get<T: Decodable>(_ path: String) async throws -> T {
        let url = try base().appendingPathComponent(path)
        let (data, resp) = try await URLSession.shared.data(from: url)
        try Self.check(resp, data)
        return try JSONDecoder().decode(T.self, from: data)
    }

    private func post<B: Encodable, T: Decodable>(_ path: String, body: B) async throws -> T {
        let url = try base().appendingPathComponent(path)
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONEncoder().encode(body)
        let (data, resp) = try await URLSession.shared.data(for: req)
        try Self.check(resp, data)
        return try JSONDecoder().decode(T.self, from: data)
    }

    private static func check(_ resp: URLResponse, _ data: Data) throws {
        guard let http = resp as? HTTPURLResponse else { return }
        if !(200..<300).contains(http.statusCode) {
            let msg = (try? JSONDecoder().decode([String: String].self, from: data))?["error"]
                ?? "HTTP \(http.statusCode)"
            throw APIError.http(msg)
        }
    }

    // Endpointy
    func apps() async throws -> [IpaItem] { try await get("api/ipa") }
    func devices() async throws -> [DeviceItem] { try await get("api/devices") }
    func jobs() async throws -> [JobItem] { try await get("api/jobs") }
    func installations() async throws -> [InstallationItem] { try await get("api/installations") }
    func account() async throws -> AccountStatus { try await get("api/account") }

    func install(deviceUDID: String, ipaID: String) async throws -> JobItem {
        try await post("api/install", body: InstallRequest(device_udid: deviceUDID, ipa_id: ipaID))
    }

    func iconURL(_ ipaID: String) -> URL? {
        try? base().appendingPathComponent("icon/\(ipaID)")
    }
}
