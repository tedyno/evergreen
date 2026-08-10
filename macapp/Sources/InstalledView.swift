import SwiftUI

/// Overview of installed apps: when signed, when the profile expires, manual resign.
struct InstalledView: View {
    @EnvironmentObject var state: AppState

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if state.installations.filter({ $0.isActive }).isEmpty {
                    ContentUnavailableView("Zatím nic nenainstalováno",
                                           systemImage: "checkmark.seal",
                                           description: Text("Nainstaluj appku v sekci Aplikace — objeví se tu s expirací profilu."))
                        .frame(maxWidth: .infinity, minHeight: 180)
                } else {
                    Text("Profily free účtu platí 7 dní. Evergreen je sám obnoví před vypršením; tady je můžeš přepodepsat i ručně.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    ForEach(state.installations.filter { $0.isActive }) { inst in
                        InstalledRow(inst: inst)
                        Divider()
                    }
                }
            }
            .padding(20)
        }
        .navigationTitle("Nainstalované")
        .task { await state.refreshInstallations() }
    }
}

struct InstalledRow: View {
    @EnvironmentObject var state: AppState
    let inst: Installation
    @State private var busy = false

    private var appName: String {
        state.ipas.first { $0.id == inst.ipaId }?.name ?? inst.signedBundleId
    }
    private var deviceName: String {
        state.devices.first { $0.udid == inst.deviceUdid }?.name ?? inst.deviceUdid
    }

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "app.badge.checkmark")
                .font(.title2)
                .foregroundStyle(.green)
                .frame(width: 40)

            VStack(alignment: .leading, spacing: 3) {
                Text(appName).fontWeight(.medium)
                HStack(spacing: 6) {
                    Text(deviceName)
                    if let d = inst.lastInstalledDate {
                        Text("· podepsáno \(dateStr(d))")
                    }
                }
                .font(.caption)
                .foregroundStyle(.secondary)
                expiryLine
            }

            Spacer()

            Button {
                busy = true
                Task {
                    try? await state.resign(ipaId: inst.ipaId, deviceUdid: inst.deviceUdid)
                    busy = false
                }
            } label: {
                if busy { ProgressView().controlSize(.small) }
                else { Label("Resign", systemImage: "arrow.triangle.2.circlepath") }
            }
            .disabled(busy)
        }
        .padding(.vertical, 4)
    }

    @ViewBuilder
    private var expiryLine: some View {
        if let exp = inst.expiryDate {
            let days = inst.daysUntilExpiry ?? 0
            let color: Color = days <= 1 ? .red : (days <= 3 ? .orange : .green)
            HStack(spacing: 6) {
                Image(systemName: "clock")
                if days <= 0 {
                    Text("Profil vypršel").foregroundStyle(.red)
                } else {
                    Text("Vyprší \(dateStr(exp)) · za \(days) \(days == 1 ? "den" : (days < 5 ? "dny" : "dní"))")
                        .foregroundStyle(color)
                }
            }
            .font(.caption)
        }
    }

    private func dateStr(_ d: Date) -> String {
        let f = DateFormatter()
        f.dateFormat = "d.M.yyyy HH:mm"
        return f.string(from: d)
    }
}
