import Foundation

/// Runs `evergreen-server` as a per-user LaunchAgent (`com.evergreen.server`), so the
/// engine — and therefore the refresh scheduler that keeps apps alive — stays up even
/// when Evergreen (the UI) is closed, and starts again at login.
///
/// The agent is installed by the app but owned by a stable, well-known plist path, so a
/// Homebrew Cask can tear it down on uninstall (`launchctl` + `delete` the plist).
@MainActor
final class ServerController: ObservableObject {
    enum State: Equatable {
        case stopped
        case starting
        case running
        case failed(String)
    }

    @Published private(set) var state: State = .stopped

    /// Port the local server listens on (loopback only).
    let port: Int = 8080

    /// Well-known LaunchAgent identity — must match the Homebrew Cask uninstall stanza.
    static let agentLabel = "com.evergreen.server"

    private let logURL: URL

    var localBaseURL: URL {
        URL(string: "http://127.0.0.1:\(port)")!
    }

    init() {
        logURL = Self.appSupportDir().appendingPathComponent("server.log")
    }

    // MARK: - paths

    static func appSupportDir() -> URL {
        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let dir = base.appendingPathComponent("evergreen", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    private static func launchAgentsDir() -> URL {
        let base = FileManager.default.urls(for: .libraryDirectory, in: .userDomainMask).first!
        let dir = base.appendingPathComponent("LaunchAgents", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    static var agentPlistURL: URL {
        launchAgentsDir().appendingPathComponent("\(agentLabel).plist")
    }

    /// The bundled server binary (in the app's Resources).
    private func serverBinaryURL() -> URL? {
        Bundle.main.url(forResource: "evergreen-server", withExtension: nil)
    }

    var isAgentInstalled: Bool {
        FileManager.default.fileExists(atPath: Self.agentPlistURL.path)
    }

    // MARK: - lifecycle

    /// Installs (or refreshes) the LaunchAgent and makes sure it's running, then waits
    /// for the health check. Idempotent — safe to call on every app launch.
    func startIfNeeded() async {
        guard state != .running && state != .starting else { return }
        guard let binary = serverBinaryURL() else {
            state = .failed("Chybí přibalená binárka serveru")
            return
        }
        state = .starting

        do {
            try writeAgentPlist(binaryPath: binary.path)
        } catch {
            state = .failed("Nelze zapsat LaunchAgent: \(error.localizedDescription)")
            return
        }
        loadAgent()

        if await waitForHealth(timeout: 15) {
            state = .running
        } else {
            state = .failed("Server nenaběhl včas — viz \(logURL.path)")
        }
    }

    /// Writes the LaunchAgent plist pointing at the bundled binary.
    private func writeAgentPlist(binaryPath: String) throws {
        let plist: [String: Any] = [
            "Label": Self.agentLabel,
            "ProgramArguments": [binaryPath],
            "EnvironmentVariables": [
                "EVERGREEN_DATA": Self.appSupportDir().path,
                "EVERGREEN_BIND": "127.0.0.1:\(port)",
                "RUST_LOG": "info",
                // No ANISETTE_URL — on macOS the native AOSKit provider wins.
            ],
            "RunAtLoad": true,
            "KeepAlive": true,
            "StandardOutPath": logURL.path,
            "StandardErrorPath": logURL.path,
            "ProcessType": "Background",
        ]
        let data = try PropertyListSerialization.data(fromPropertyList: plist, format: .xml, options: 0)
        try data.write(to: Self.agentPlistURL)
    }

    /// (Re)loads the agent so it picks up the current plist.
    private func loadAgent() {
        let domain = "gui/\(getuid())"
        let target = "\(domain)/\(Self.agentLabel)"
        // Boot out any previous instance so bootstrap can re-read the plist.
        _ = Self.runLaunchctl(["bootout", target])
        _ = Self.runLaunchctl(["bootstrap", domain, Self.agentPlistURL.path])
        _ = Self.runLaunchctl(["enable", target])
        _ = Self.runLaunchctl(["kickstart", "-k", target])
    }

    /// Stops and removes the background agent entirely (used by the Settings toggle and
    /// mirrored by the Homebrew Cask uninstall).
    func uninstallAgent() {
        _ = Self.runLaunchctl(["bootout", "gui/\(getuid())/\(Self.agentLabel)"])
        try? FileManager.default.removeItem(at: Self.agentPlistURL)
        state = .stopped
    }

    /// No-op on app termination: the whole point is that the agent keeps running so the
    /// refresh scheduler survives the UI being closed.
    func stop() {}

    @discardableResult
    private static func runLaunchctl(_ args: [String]) -> Int32 {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/bin/launchctl")
        p.arguments = args
        do {
            try p.run()
            p.waitUntilExit()
            return p.terminationStatus
        } catch {
            return -1
        }
    }

    private func waitForHealth(timeout seconds: Double) async -> Bool {
        let deadline = Date().addingTimeInterval(seconds)
        let url = localBaseURL.appendingPathComponent("api/status")
        while Date() < deadline {
            var req = URLRequest(url: url)
            req.timeoutInterval = 2
            if let (_, resp) = try? await URLSession.shared.data(for: req),
               let http = resp as? HTTPURLResponse, http.statusCode == 200 {
                return true
            }
            try? await Task.sleep(nanoseconds: 400_000_000)
        }
        return false
    }
}
