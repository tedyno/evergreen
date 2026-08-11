import SwiftUI

enum Section: String, CaseIterable, Identifiable {
    case overview = "Přehled"
    case apps = "Aplikace"
    case devices = "Zařízení"
    case jobs = "Úlohy"
    case account = "Účet"

    var id: String { rawValue }

    var icon: String {
        switch self {
        case .overview: return "house"
        case .apps: return "square.grid.2x2"
        case .devices: return "ipad.and.iphone"
        case .jobs: return "list.bullet.rectangle"
        case .account: return "person.crop.circle"
        }
    }

    /// Localized title for display in the sidebar.
    @MainActor
    func title(_ s: AppState) -> String {
        switch self {
        case .overview: return s.t("Přehled", "Overview")
        case .apps: return s.t("Aplikace", "Apps")
        case .devices: return s.t("Zařízení", "Devices")
        case .jobs: return s.t("Úlohy", "Jobs")
        case .account: return s.t("Účet", "Account")
        }
    }
}

struct ContentView: View {
    @EnvironmentObject var state: AppState
    @EnvironmentObject var server: ServerController
    @State private var selection: Section? = .overview

    var body: some View {
        if state.initialLoadDone {
            splitView
        } else {
            loadingScreen
        }
    }

    /// Shown while the server is coming up and the first data load runs — avoids briefly
    /// flashing "not connected" / "not logged in" before we actually know the state.
    private var loadingScreen: some View {
        VStack(spacing: 16) {
            ProgressView().controlSize(.large)
            Text(loadingMessage)
                .font(.callout)
                .foregroundStyle(.secondary)
        }
        .frame(minWidth: 760, minHeight: 500)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .windowBackgroundColor))
        .navigationTitle("Evergreen")
    }

    private var loadingMessage: String {
        switch server.state {
        case .starting: return state.t("Spouštím server…", "Starting server…")
        case .failed(let m): return state.t("Chyba serveru: \(m)", "Server error: \(m)")
        default: return state.t("Načítám…", "Loading…")
        }
    }

    private var splitView: some View {
        NavigationSplitView {
            List(Section.allCases, selection: $selection) { section in
                HStack {
                    Label(section.title(state), systemImage: section.icon)
                    if section == .jobs && state.hasActiveJob {
                        Spacer()
                        ProgressView().controlSize(.small)
                    }
                }
                .tag(section)
            }
            .navigationSplitViewColumnWidth(min: 170, ideal: 190, max: 240)
            .safeAreaInset(edge: .bottom) {
                ConnectionStatusBar()
            }
        } detail: {
            switch selection ?? .overview {
            case .overview: OverviewView()
            case .apps: AppsView()
            case .devices: DevicesView()
            case .jobs: JobsView()
            case .account: AccountView()
            }
        }
        .navigationTitle("Evergreen")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    Task { await state.refreshAll() }
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .help(state.t("Obnovit", "Refresh"))
            }
        }
    }
}

/// Bottom bar of the sidebar: connection status + server version.
struct ConnectionStatusBar: View {
    @EnvironmentObject var state: AppState

    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(state.status != nil ? Color.green : Color.red)
                .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 1) {
                if let s = state.status {
                    Text(state.t("Připojeno · v\(s.version)", "Connected · v\(s.version)"))
                        .font(.caption)
                } else {
                    Text(state.t("Nepřipojeno", "Not connected"))
                        .font(.caption)
                }
                Text(state.baseURL.absoluteString)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer()
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background(.bar)
    }
}
