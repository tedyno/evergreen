import SwiftUI
import AppKit

@main
struct HomesignApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var state = AppState()
    @StateObject private var server = ServerController()
    @StateObject private var pair = PairService()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(state)
                .environmentObject(server)
                .environmentObject(pair)
                .frame(minWidth: 760, minHeight: 500)
                .task {
                    appDelegate.server = server
                    await bootstrap()
                }
        }
        .windowStyle(.titleBar)
        .commands {
            CommandGroup(replacing: .newItem) {}
        }

        Settings {
            SettingsView()
                .environmentObject(state)
                .environmentObject(server)
        }
    }

    /// Podle režimu buď spustí vlastní server, nebo se připojí na vzdálený.
    private func bootstrap() async {
        if state.useLocalServer {
            await server.startIfNeeded()
            await state.activate(baseURL: server.localBaseURL)
        } else {
            await state.switchToRemote()
        }
        state.startPolling()
    }
}

/// Zajišťuje, že se embedded server při zavření appky korektně ukončí.
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var server: ServerController?

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationWillTerminate(_ notification: Notification) {
        server?.stop()
    }
}
