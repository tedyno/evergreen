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

    /// False until the first full load finishes — the UI shows a loading screen until then,
    /// so we don't briefly flash "not logged in" / empty states while data is still coming in.
    @Published var initialLoadDone = false

    private(set) var client: ApiClient

    private var pollTask: Task<Void, Never>?
    // Baselines for change detection → notifications.
    private var lastJobStatus: [Int64: String] = [:]
    private var lastAuthState: String?
    private var notificationsPrimed = false

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
        initialLoadDone = true
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

    // MARK: - health summary (dashboard + menu bar)

    /// Installed apps currently being kept alive.
    var activeInstallations: [Installation] { installations.filter { $0.isActive } }

    /// Fewest whole days until any app's profile expires (nil if unknown / none).
    var soonestExpiryDays: Int? { activeInstallations.compactMap { $0.daysUntilExpiry }.min() }

    /// Things needing attention: an expired app, a stuck login, or the server being down.
    var issueCount: Int {
        var n = 0
        if status == nil { n += 1 }
        if let a = account?.authState, a != "logged_in" { n += 1 }
        n += activeInstallations.filter { ($0.daysUntilExpiry ?? 99) <= 0 }.count
        return n
    }

    var isHealthy: Bool { issueCount == 0 }

    func cancelJob(_ id: Int64) async {
        try? await client.cancelJob(id: id)
        await refreshJobs()
    }

    private var lastAppIdAttempt: Date?

    /// Loads the actual App ID state from the Apple account (only when logged in). The
    /// first call obtains and caches the Xcode token (valid ~a year); after that it's
    /// cheap. (The earlier "-22411" wasn't a throttle but a stale session, now auto-healed.)
    func refreshAppIds(force: Bool = false) async {
        guard account?.authState == "logged_in" else { appIdInfo = nil; return }
        // Load once per session; refresh only on explicit request (force).
        if appIdInfo != nil && !force { return }
        // Light guard against rapid repeats if a load fails (except on manual force).
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

    /// Re-signs and reinstalls every active app now (enqueues one job each).
    func refreshAllApps() async {
        for inst in activeInstallations {
            _ = try? await client.install(deviceUdid: inst.deviceUdid, ipaId: inst.ipaId)
        }
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

    /// Starts periodic loading (runs for the lifetime of the app). Jobs every 2 s;
    /// account/installations less often; then checks for anything worth a notification.
    func startPolling() {
        guard pollTask == nil else { return }
        pollTask = Task { [weak self] in
            var tick = 0
            while !Task.isCancelled {
                await self?.refreshJobs()
                if tick % 3 == 0 {
                    await self?.refreshInstallations()
                    await self?.refreshAccount()
                }
                await self?.checkForNotifications()
                tick += 1
                try? await Task.sleep(nanoseconds: 2_000_000_000)
            }
        }
    }

    /// Diffs against the previous poll and posts notifications for new problems /
    /// finished automatic renewals. The first pass only records baselines (no spam).
    private func checkForNotifications() {
        let primed = notificationsPrimed

        // Login needs the user (e.g. 2FA) — surface it once on transition.
        if primed, let a = account?.authState, a != "logged_in", a != lastAuthState {
            NotificationManager.shared.post(
                title: t("Evergreen: přihlášení", "Evergreen: sign-in"),
                body: t("Apple ID vyžaduje ověření — otevři Účet a přihlas se.",
                        "Your Apple ID needs verification — open Account and sign in."))
        }
        lastAuthState = account?.authState

        for job in jobs {
            let prev = lastJobStatus[job.id]
            let appName = ipas.first { $0.id == job.ipaId }?.name ?? ""
            if primed, prev == nil, job.kind == "refresh", job.status == "blocked" {
                // Scheduler wanted to renew but the iPad is locked.
                NotificationManager.shared.post(
                    title: t("Evergreen: odemkni iPad", "Evergreen: unlock your iPad"),
                    body: appName.isEmpty
                        ? t("iPad je zamčený — odemkni ho pro obnovu.", "Your iPad is locked — unlock it to renew.")
                        : t("iPad je zamčený — odemkni ho pro obnovu \(appName).", "Your iPad is locked — unlock it to renew \(appName)."))
            } else if primed, prev == nil, job.kind == "refresh", job.status == "queued" || job.status == "running" {
                // Automatic renewal just started.
                NotificationManager.shared.post(
                    title: t("Evergreen: obnova", "Evergreen: renewal"),
                    body: appName.isEmpty
                        ? t("Zahajuji obnovu…", "Starting renewal…")
                        : t("Zahajuji obnovu \(appName)…", "Renewing \(appName)…"))
            } else if primed, job.status == "error", prev != "error" {
                let title = job.kind == "refresh"
                    ? t("Evergreen: obnova selhala", "Evergreen: renewal failed")
                    : t("Evergreen: instalace selhala", "Evergreen: install failed")
                NotificationManager.shared.post(
                    title: title,
                    body: job.message ?? t("Podpis/instalace se nezdařila.", "Signing/install failed."))
            } else if primed, job.status == "done", prev != "done", job.kind == "refresh" {
                NotificationManager.shared.post(
                    title: t("Evergreen: appka obnovena", "Evergreen: app renewed"),
                    body: appName.isEmpty
                        ? t("Profil byl obnoven.", "The profile was renewed.")
                        : t("\(appName): profil obnoven.", "\(appName): profile renewed."))
            }
            lastJobStatus[job.id] = job.status
        }
        notificationsPrimed = true
    }

    func stopPolling() {
        pollTask?.cancel()
        pollTask = nil
    }
}
