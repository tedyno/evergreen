import SwiftUI

/// Overview of installed apps: when signed, when the profile expires, manual resign.
struct InstalledView: View {
    @EnvironmentObject var state: AppState

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if state.installations.filter({ $0.isActive }).isEmpty {
                    ContentUnavailableView(state.t("Zatím nic nenainstalováno", "Nothing installed yet"),
                                           systemImage: "checkmark.seal",
                                           description: Text(state.t("Nainstaluj appku v sekci Aplikace — objeví se tu s expirací profilu.", "Install an app in the Apps section — it will appear here with its profile expiry.")))
                        .frame(maxWidth: .infinity, minHeight: 180)
                } else {
                    Text(state.t("Profily free účtu platí 7 dní. Evergreen je sám obnoví před vypršením; tady je můžeš přepodepsat i ručně.", "Free-account profiles are valid for 7 days. Evergreen renews them automatically before they expire; here you can also re-sign them manually."))
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
        .navigationTitle(state.t("Nainstalované", "Installed"))
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
                        Text(state.t("· podepsáno \(dateStr(d))", "· signed \(dateStr(d))"))
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
                    Text(state.t("Profil vypršel", "Profile expired")).foregroundStyle(.red)
                } else {
                    Text(state.t("Vyprší \(dateStr(exp)) · za \(days) \(days == 1 ? "den" : (days < 5 ? "dny" : "dní"))", "Expires \(dateStr(exp)) · in \(days) \(days == 1 ? "day" : "days")"))
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
