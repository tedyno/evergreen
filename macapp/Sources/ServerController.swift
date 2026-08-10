import Foundation

/// Launches and supervises the bundled `homesign-server` as a subprocess. Lets you keep
/// "everything inside the Mac app" — no Docker, no manual terminal.
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

    private var process: Process?
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
        let dir = base.appendingPathComponent("homesign", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    /// The bundled server binary (in the app's Resources).
    private func serverBinaryURL() -> URL? {
        Bundle.main.url(forResource: "homesign-server", withExtension: nil)
    }

    // MARK: - lifecycle

    /// Starts the server (if not already running) and waits until the health check responds.
    func startIfNeeded() async {
        guard state != .running && state != .starting else { return }
        guard let binary = serverBinaryURL() else {
            state = .failed("Chybí přibalená binárka serveru")
            return
        }
        state = .starting

        let dataDir = Self.appSupportDir()
        let proc = Process()
        proc.executableURL = binary
        proc.environment = [
            "HOMESIGN_DATA": dataDir.path,
            "HOMESIGN_BIND": "127.0.0.1:\(port)",
            "RUST_LOG": "info",
            // We don't set ANISETTE_URL — on macOS the native AOSKit provider wins.
        ]

        // Redirect the server's logs to a file.
        FileManager.default.createFile(atPath: logURL.path, contents: nil)
        if let handle = try? FileHandle(forWritingTo: logURL) {
            proc.standardOutput = handle
            proc.standardError = handle
        }

        proc.terminationHandler = { [weak self] p in
            Task { @MainActor in
                guard let self else { return }
                if self.state == .running || self.state == .starting {
                    self.state = .failed("Server skončil (kód \(p.terminationStatus))")
                }
            }
        }

        do {
            try proc.run()
            process = proc
        } catch {
            state = .failed("Nelze spustit server: \(error.localizedDescription)")
            return
        }

        // Health check: wait until the server responds on /api/status.
        if await waitForHealth(timeout: 15) {
            state = .running
        } else {
            state = .failed("Server nenaběhl včas — viz \(logURL.path)")
            stop()
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

    /// Stops the server (SIGTERM). Called when the app terminates.
    func stop() {
        guard let proc = process, proc.isRunning else { return }
        proc.terminationHandler = nil
        proc.terminate()
        process = nil
        if state == .running || state == .starting {
            state = .stopped
        }
    }
}
