import Foundation

/// An error with a human-readable message taken from the server's response body.
struct ApiError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

/// A thin async client on top of the homesign server's REST API.
actor ApiClient {
    private var baseURL: URL

    init(baseURL: URL) {
        self.baseURL = baseURL
    }

    func setBaseURL(_ url: URL) {
        baseURL = url
    }

    private func url(_ path: String) -> URL {
        baseURL.appendingPathComponent(path)
    }

    // MARK: - generic requests

    private func send(_ request: URLRequest) async throws -> Data {
        let (data, response): (Data, URLResponse)
        do {
            (data, response) = try await URLSession.shared.data(for: request)
        } catch {
            throw ApiError(message: "Nedostupný server: \(error.localizedDescription)")
        }
        guard let http = response as? HTTPURLResponse else {
            throw ApiError(message: "Neplatná odpověď serveru")
        }
        guard (200..<300).contains(http.statusCode) else {
            // The server returns errors as {"error": "..."}.
            if let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let msg = obj["error"] as? String {
                throw ApiError(message: msg)
            }
            throw ApiError(message: "HTTP \(http.statusCode)")
        }
        return data
    }

    private func get<T: Decodable>(_ path: String, as type: T.Type) async throws -> T {
        let data = try await send(URLRequest(url: url(path)))
        return try JSONDecoder().decode(T.self, from: data)
    }

    private func post<T: Decodable>(_ path: String, json body: [String: Any], as type: T.Type) async throws -> T {
        var req = URLRequest(url: url(path))
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.httpBody = try JSONSerialization.data(withJSONObject: body)
        let data = try await send(req)
        return try JSONDecoder().decode(T.self, from: data)
    }

    @discardableResult
    private func postNoBody(_ path: String) async throws -> Data {
        var req = URLRequest(url: url(path))
        req.httpMethod = "POST"
        return try await send(req)
    }

    @discardableResult
    private func delete(_ path: String) async throws -> Data {
        var req = URLRequest(url: url(path))
        req.httpMethod = "DELETE"
        return try await send(req)
    }

    // MARK: - endpoints

    func status() async throws -> ServerStatus {
        try await get("api/status", as: ServerStatus.self)
    }

    func account() async throws -> Account {
        try await get("api/account", as: Account.self)
    }

    func login(appleId: String, password: String) async throws -> AuthOutcome {
        try await post("api/account/login", json: ["apple_id": appleId, "password": password], as: AuthOutcome.self)
    }

    func submit2FA(code: String) async throws -> AuthOutcome {
        try await post("api/account/2fa", json: ["code": code], as: AuthOutcome.self)
    }

    func logout() async throws {
        try await postNoBody("api/account/logout")
    }

    func devices() async throws -> [Device] {
        try await get("api/devices", as: [Device].self)
    }

    func deleteDevice(udid: String) async throws {
        try await delete("api/devices/\(udid)")
    }

    func setDeviceAddress(udid: String, address: String) async throws {
        _ = try await post("api/devices/\(udid)/address", json: ["address": address], as: OkResponse.self)
    }

    /// Re-detects the device's IP via usbmuxd. Returns the found address (or nil).
    func detectDeviceIP(udid: String) async throws -> String? {
        struct AddrResponse: Decodable { let address: String? }
        var req = URLRequest(url: url("api/devices/\(udid)/detect-ip"))
        req.httpMethod = "POST"
        req.timeoutInterval = 20
        let data = try await send(req)
        return (try? JSONDecoder().decode(AddrResponse.self, from: data))?.address
    }

    func ipas() async throws -> [Ipa] {
        try await get("api/ipa", as: [Ipa].self)
    }

    func deleteIpa(id: String) async throws {
        try await delete("api/ipa/\(id)")
    }

    func jobs() async throws -> [HSJob] {
        try await get("api/jobs", as: [HSJob].self)
    }

    func installations() async throws -> [Installation] {
        try await get("api/installations", as: [Installation].self)
    }

    func cancelJob(id: Int64) async throws {
        var req = URLRequest(url: url("api/jobs/\(id)/cancel"))
        req.httpMethod = "POST"
        _ = try await send(req)
    }

    /// The actual App ID state from the Apple account (slow — round-trip to Apple).
    func accountAppIds() async throws -> AppIdInfo {
        var req = URLRequest(url: url("api/appids"))
        req.timeoutInterval = 60
        let data = try await send(req)
        return try JSONDecoder().decode(AppIdInfo.self, from: data)
    }

    func install(deviceUdid: String, ipaId: String) async throws -> HSJob {
        try await post("api/install", json: ["device_udid": deviceUdid, "ipa_id": ipaId], as: HSJob.self)
    }

    /// IPA icon URL for AsyncImage.
    func iconURL(for ipaId: String) -> URL {
        url("icon/\(ipaId)")
    }

    /// Uploads the .ipa via multipart with progress reporting.
    func uploadIpa(fileURL: URL, progress: @escaping @Sendable (Double) -> Void) async throws -> Ipa {
        let boundary = "homesign.\(UUID().uuidString)"
        var req = URLRequest(url: url("api/ipa"))
        req.httpMethod = "POST"
        req.setValue("multipart/form-data; boundary=\(boundary)", forHTTPHeaderField: "Content-Type")

        let filename = fileURL.lastPathComponent
        let fileData = try Data(contentsOf: fileURL)

        var body = Data()
        body.append("--\(boundary)\r\n".data(using: .utf8)!)
        body.append("Content-Disposition: form-data; name=\"file\"; filename=\"\(filename)\"\r\n".data(using: .utf8)!)
        body.append("Content-Type: application/octet-stream\r\n\r\n".data(using: .utf8)!)
        body.append(fileData)
        body.append("\r\n--\(boundary)--\r\n".data(using: .utf8)!)

        let delegate = UploadProgressDelegate(onProgress: progress)
        let session = URLSession(configuration: .default, delegate: delegate, delegateQueue: nil)
        let (data, response) = try await session.upload(for: req, from: body)
        guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
            if let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
               let msg = obj["error"] as? String {
                throw ApiError(message: msg)
            }
            let code = (response as? HTTPURLResponse)?.statusCode ?? -1
            throw ApiError(message: "Upload selhal (HTTP \(code))")
        }
        return try JSONDecoder().decode(Ipa.self, from: data)
    }
}

/// Delegate for upload progress.
private final class UploadProgressDelegate: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    let onProgress: @Sendable (Double) -> Void
    init(onProgress: @escaping @Sendable (Double) -> Void) { self.onProgress = onProgress }

    func urlSession(_ session: URLSession, task: URLSessionTask,
                    didSendBodyData bytesSent: Int64, totalBytesSent: Int64,
                    totalBytesExpectedToSend total: Int64) {
        guard total > 0 else { return }
        onProgress(Double(totalBytesSent) / Double(total))
    }
}
