import Foundation
import SwiftUI

/// Shared app state: server configuration, loaded data, job polling.
@MainActor
final class AppState: ObservableObject {
    /// UI language: "system" | "cs" | "en". Switches instantly (views observe AppState).
    @Published var appLanguage: String = UserDefaults.standard.string(forKey: "appLanguage") ?? "system" {
        didSet { UserDefaults.standard.set(appLanguage, forKey: "appLanguage") }
    }

    /// Picks the string for the current UI language.
    func t(_ cs: String, _ en: String) -> String {
        let lang: String
        if appLanguage == "cs" || appLanguage == "en" {
            lang = appLanguage
        } else {
            lang = (Locale.preferredLanguages.first ?? "en").hasPrefix("cs") ? "cs" : "en"
        }
        return lang == "cs" ? cs : en
    }

    /// Use the embedded server (default), or connect to a remote one?
    @AppStorage("useLocalServer") var useLocalServer: Bool = true
    /// Address of the remote server (only when useLocalServer == false).
    @AppStorage("remoteServerURL") var remoteServerURLString: String = "http://localhost:8080"

    /// The address the client is currently pointing at.
    @Published private(set) var activeBaseURL: URL

    @Published var status: ServerStatus?
    @Published var account: Account?
    @Published var devices: [Device] = []
    @Published var ipas: [Ipa] = []
    @Published var jobs: [HSJob] = []
    @Published var installations: [Installation] = []
    @Published var appIdInfo: AppIdInfo?
    @Published var appIdLoading = false

    @Published var connectionError: String?
    @Published var uploadProgress: Double?

    private(set) var client: ApiClient

    private var pollTask: Task<Void, Never>?

    init() {
        let url = URL(string: "http://127.0.0.1:8080")!
        activeBaseURL = url
        client = ApiClient(baseURL: url)
    }

    var baseURL: URL { activeBaseURL }

    /// Synchronous IPA icon URL for `AsyncImage`.
    func iconURLSync(for ipa: Ipa) -> URL {
        activeBaseURL.appendingPathComponent("icon/\(ipa.id)")
    }

    /// Redirects the client to the given address and loads data.
    func activate(baseURL url: URL) async {
        activeBaseURL = url
        await client.setBaseURL(url)
        await refreshAll()
    }

    /// Switches to the remote server using the stored address.
    func switchToRemote() async {
        useLocalServer = false
        let url = URL(string: remoteServerURLString) ?? URL(string: "http://localhost:8080")!
        await activate(baseURL: url)
    }

    // MARK: - loading

    func refreshAll() async {
        await refreshStatus()
        await refreshAccount()
        await refreshDevices()
        await refreshIpas()
        await refreshJobs()
        await refreshInstallations()
    }

    func refreshInstallations() async {
        if let i = try? await client.installations() { installations = i }
    }

    /// Is any job running? (for the loading indicator in the menu)
    var hasActiveJob: Bool { jobs.contains { $0.isActive } }

    func cancelJob(_ id: Int64) async {
        try? await client.cancelJob(id: id)
        await refreshJobs()
    }

    private var lastAppIdAttempt: Date?

    /// Loads the actual App ID state from the Apple account (only when logged in).
    /// Safeguard: don't hit the Apple token endpoint often — otherwise throttle -22411 may occur.
    func refreshAppIds(force: Bool = false) async {
        guard account?.authState == "logged_in" else { appIdInfo = nil; return }
        // If we already have data, don't reload automatically. Retry only manually (force).
        if appIdInfo != nil && !force { return }
        // Don't retry more often than once every 5 minutes (except on manual force).
        if let last = lastAppIdAttempt, !force, Date().timeIntervalSince(last) < 300 { return }
        lastAppIdAttempt = Date()
        appIdLoading = true
        defer { appIdLoading = false }
        appIdInfo = try? await client.accountAppIds()
    }

    func refreshStatus() async {
        do {
            status = try await client.status()
            connectionError = nil
        } catch {
            status = nil
            connectionError = (error as? ApiError)?.message ?? error.localizedDescription
        }
    }

    func refreshAccount() async {
        account = try? await client.account()
    }

    func refreshDevices() async {
        if let d = try? await client.devices() { devices = d }
    }

    func refreshIpas() async {
        if let i = try? await client.ipas() { ipas = i }
    }

    func refreshJobs() async {
        if let j = try? await client.jobs() { jobs = j }
    }

    // MARK: - actions

    func uploadIpa(fileURL: URL) async throws {
        uploadProgress = 0
        defer { uploadProgress = nil }
        _ = try await client.uploadIpa(fileURL: fileURL) { [weak self] p in
            Task { @MainActor in self?.uploadProgress = p }
        }
        await refreshIpas()
    }

    func deleteIpa(_ ipa: Ipa) async {
        try? await client.deleteIpa(id: ipa.id)
        await refreshIpas()
    }

    func deleteDevice(_ device: Device) async {
        try? await client.deleteDevice(udid: device.udid)
        await refreshDevices()
    }

    func setDeviceAddress(udid: String, address: String) async throws {
        try await client.setDeviceAddress(udid: udid, address: address)
        await refreshDevices()
    }

    @discardableResult
    func detectDeviceIP(udid: String) async -> String? {
        let ip = try? await client.detectDeviceIP(udid: udid)
        await refreshDevices()
        return ip ?? nil
    }

    func install(ipa: Ipa, onDevice udid: String) async throws {
        _ = try await client.install(deviceUdid: udid, ipaId: ipa.id)
        await refreshJobs()
    }

    /// Re-sign + reinstall an already installed app (manual refresh).
    func resign(ipaId: String, deviceUdid: String) async throws {
        _ = try await client.install(deviceUdid: deviceUdid, ipaId: ipaId)
        await refreshJobs()
    }

    func login(appleId: String, password: String) async throws -> AuthOutcome {
        let outcome = try await client.login(appleId: appleId, password: password)
        await refreshAccount()
        return outcome
    }

    func submit2FA(code: String) async throws -> AuthOutcome {
        let outcome = try await client.submit2FA(code: code)
        await refreshAccount()
        return outcome
    }

    func logout() async {
        try? await client.logout()
        await refreshAccount()
    }

    // MARK: - polling

    /// Starts periodic job loading (runs for the lifetime of the app).
    func startPolling() {
        guard pollTask == nil else { return }
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                await self?.refreshJobs()
                try? await Task.sleep(nanoseconds: 2_000_000_000)
            }
        }
    }

    func stopPolling() {
        pollTask?.cancel()
        pollTask = nil
    }
}
