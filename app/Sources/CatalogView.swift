import SwiftUI

/// Katalog nahraných IPA — tapnutím vybereš zařízení a pošleš serveru příkaz
/// k instalaci. Refresh (7denní obnova) běží na serveru, appka ho neřeší.
struct CatalogView: View {
    @EnvironmentObject var api: API
    @State private var apps: [IpaItem] = []
    @State private var devices: [DeviceItem] = []
    @State private var error: String?
    @State private var loading = false
    @State private var installTarget: IpaItem?

    var body: some View {
        NavigationStack {
            Group {
                if apps.isEmpty && !loading {
                    ContentUnavailableView(
                        "Žádné aplikace",
                        systemImage: "tray",
                        description: Text("Nahraj IPA přes web UI serveru.")
                    )
                } else {
                    List(apps) { app in
                        AppRow(app: app)
                            .contentShape(Rectangle())
                            .onTapGesture { installTarget = app }
                    }
                }
            }
            .navigationTitle("homesign")
            .refreshable { await load() }
            .task { await load() }
            .overlay(alignment: .bottom) { errorBar }
            .confirmationDialog(
                "Instalovat \(installTarget?.name ?? "")",
                isPresented: Binding(get: { installTarget != nil },
                                     set: { if !$0 { installTarget = nil } }),
                titleVisibility: .visible
            ) {
                ForEach(devices) { dev in
                    Button(dev.name) { install(app: installTarget, on: dev) }
                }
                Button("Zrušit", role: .cancel) { installTarget = nil }
            } message: {
                Text(devices.isEmpty ? "Nejdřív spáruj zařízení přes CLI." : "Vyber zařízení")
            }
        }
    }

    @ViewBuilder private var errorBar: some View {
        if let error {
            Text(error)
                .font(.footnote).foregroundStyle(.white)
                .padding(10).background(.red, in: Capsule())
                .padding(.bottom, 8)
        }
    }

    private func load() async {
        loading = true; defer { loading = false }
        do {
            async let a = api.apps()
            async let d = api.devices()
            apps = try await a
            devices = try await d
            error = nil
        } catch { self.error = error.localizedDescription }
    }

    private func install(app: IpaItem?, on device: DeviceItem) {
        guard let app else { return }
        installTarget = nil
        Task {
            do { _ = try await api.install(deviceUDID: device.udid, ipaID: app.id) }
            catch { self.error = error.localizedDescription }
        }
    }
}

private struct AppRow: View {
    @EnvironmentObject var api: API
    let app: IpaItem
    var body: some View {
        HStack(spacing: 12) {
            AsyncImage(url: api.iconURL(app.id)) { img in
                img.resizable().aspectRatio(contentMode: .fill)
            } placeholder: {
                RoundedRectangle(cornerRadius: 10).fill(.quaternary)
            }
            .frame(width: 48, height: 48)
            .clipShape(RoundedRectangle(cornerRadius: 10))
            VStack(alignment: .leading, spacing: 2) {
                Text(app.name).font(.headline)
                Text(app.bundle_id).font(.caption).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
            }
            Spacer()
            if let v = app.version {
                Text(v).font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 4)
    }
}
