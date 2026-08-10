import SwiftUI
import AppKit

@main
struct HomesignApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @StateObject private var state = AppState()
    @StateObject private var server = ServerController()
    @StateObject private var pair = PairService()

    /// Show the Evergreen icon in the macOS menu bar (default on).
    @AppStorage("showMenuBarIcon") private var showMenuBarIcon = true

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

        // Menu bar icon (toggleable in Settings).
        MenuBarExtra(isInserted: $showMenuBarIcon) {
            MenuBarContent()
                .environmentObject(state)
                .environmentObject(server)
        } label: {
            Image(nsImage: Self.menuBarIcon)
        }

        Settings {
            SettingsView()
                .environmentObject(state)
                .environmentObject(server)
        }
    }

    /// Menu bar template image (macOS tints it for light/dark).
    static var menuBarIcon: NSImage {
        let img = NSImage(named: "menubar") ?? NSImage(size: NSSize(width: 18, height: 18))
        img.isTemplate = true
        return img
    }

    /// Depending on the mode, either starts the embedded server or connects to a remote one.
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

/// Menu bar dropdown: status + basic actions.
struct MenuBarContent: View {
    @EnvironmentObject var state: AppState

    var body: some View {
        if let s = state.status {
            Text("Připojeno · v\(s.version)")
        } else {
            Text("Nepřipojeno")
        }
        if state.hasActiveJob {
            Text("Probíhá úloha…")
        }
        Divider()
        Button("Otevřít Evergreen") {
            NSApp.activate(ignoringOtherApps: true)
        }
        Button("Ukončit") {
            NSApplication.shared.terminate(nil)
        }
    }
}

/// Ensures the embedded server is shut down cleanly when the app closes.
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var server: ServerController?

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationWillTerminate(_ notification: Notification) {
        server?.stop()
    }
}
