import SwiftUI

enum Section: String, CaseIterable, Identifiable {
    case apps = "Aplikace"
    case installed = "Nainstalované"
    case devices = "Zařízení"
    case jobs = "Úlohy"
    case account = "Účet"

    var id: String { rawValue }

    var icon: String {
        switch self {
        case .apps: return "square.grid.2x2"
        case .installed: return "checkmark.seal"
        case .devices: return "ipad.and.iphone"
        case .jobs: return "list.bullet.rectangle"
        case .account: return "person.crop.circle"
        }
    }

    /// Localized title for display in the sidebar.
    @MainActor
    func title(_ s: AppState) -> String {
        switch self {
        case .apps: return s.t("Aplikace", "Apps")
        case .installed: return s.t("Nainstalované", "Installed")
        case .devices: return s.t("Zařízení", "Devices")
        case .jobs: return s.t("Úlohy", "Jobs")
        case .account: return s.t("Účet", "Account")
        }
    }
}

struct ContentView: View {
    @EnvironmentObject var state: AppState
    @State private var selection: Section? = .apps

    var body: some View {
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
            switch selection ?? .apps {
            case .apps: AppsView()
            case .installed: InstalledView()
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
