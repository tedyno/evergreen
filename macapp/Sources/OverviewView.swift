import SwiftUI

/// Home dashboard: at-a-glance health, key stats, "refresh all", and the list of apps
/// being kept alive (with per-app expiry + manual resign).
struct OverviewView: View {
    @EnvironmentObject var state: AppState
    @State private var refreshingAll = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                healthBanner
                statTiles

                if !state.activeInstallations.isEmpty {
                    HStack {
                        Text(state.t("Udržované aplikace", "Apps kept alive")).font(.headline)
                        Spacer()
                        Button {
                            refreshingAll = true
                            Task { await state.refreshAllApps(); refreshingAll = false }
                        } label: {
                            if refreshingAll { ProgressView().controlSize(.small) }
                            else { Label(state.t("Obnovit vše", "Refresh all"), systemImage: "arrow.triangle.2.circlepath") }
                        }
                        .disabled(refreshingAll)
                    }

                    Text(state.t("Profily free účtu platí 7 dní. Evergreen je sám obnoví před vypršením; tady je můžeš přepodepsat i ručně.", "Free-account profiles are valid for 7 days. Evergreen renews them automatically before they expire; here you can also re-sign them manually."))
                        .font(.caption).foregroundStyle(.secondary)

                    ForEach(state.activeInstallations) { inst in
                        InstalledRow(inst: inst)
                        Divider()
                    }
                } else {
                    ContentUnavailableView(state.t("Zatím nic nenainstalováno", "Nothing installed yet"),
                                           systemImage: "checkmark.seal",
                                           description: Text(state.t("Nainstaluj appku v sekci Aplikace — objeví se tu s expirací profilu.", "Install an app in the Apps section — it will appear here with its profile expiry.")))
                        .frame(maxWidth: .infinity, minHeight: 160)
                }
            }
            .padding(20)
        }
        .navigationTitle(state.t("Přehled", "Overview"))
        .task { await state.refreshInstallations() }
    }

    private var healthBanner: some View {
        let ok = state.isHealthy
        return HStack(spacing: 12) {
            Image(systemName: ok ? "checkmark.circle.fill" : "exclamationmark.triangle.fill")
                .font(.title)
                .foregroundStyle(ok ? .green : .orange)
            VStack(alignment: .leading, spacing: 2) {
                Text(ok ? state.t("Vše v pořádku", "All good")
                        : state.t("Vyžaduje pozornost (\(state.issueCount))", "Needs attention (\(state.issueCount))"))
                    .font(.headline)
                Text(ok
                     ? state.t("Aplikace jsou naživu a obnovují se automaticky.", "Your apps are alive and renew automatically.")
                     : state.t("Zkontroluj přihlášení, dostupnost iPadu a Úlohy.", "Check your sign-in, the iPad's reachability, and Jobs."))
                    .font(.caption).foregroundStyle(.secondary)
            }
            Spacer()
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 12).fill((ok ? Color.green : Color.orange).opacity(0.12)))
    }

    private var statTiles: some View {
        HStack(spacing: 12) {
            StatTile(icon: "app.badge.checkmark",
                     value: "\(state.activeInstallations.count)",
                     label: state.t("Naživu", "Alive"), tint: .green)
            StatTile(icon: "clock",
                     value: expiryValue, label: state.t("Nejbližší vypršení", "Soonest expiry"), tint: expiryTint)
            StatTile(icon: "server.rack",
                     value: state.status != nil ? "●" : "○",
                     label: state.t("Server", "Server"), tint: state.status != nil ? .green : .red)
            StatTile(icon: "person.crop.circle",
                     value: state.account?.authState == "logged_in" ? "●" : "○",
                     label: "Apple ID", tint: state.account?.authState == "logged_in" ? .green : .orange)
        }
    }

    private var expiryValue: String {
        guard let d = state.soonestExpiryDays else { return "—" }
        if d <= 0 { return state.t("vypršel", "expired") }
        return state.t("\(d) d", "\(d)d")
    }
    private var expiryTint: Color {
        guard let d = state.soonestExpiryDays else { return .secondary }
        return d <= 1 ? .red : (d <= 3 ? .orange : .green)
    }
}

/// A small labelled metric tile for the dashboard.
struct StatTile: View {
    let icon: String
    let value: String
    let label: String
    let tint: Color

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Image(systemName: icon).foregroundStyle(tint)
            Text(value).font(.title2.weight(.semibold)).foregroundStyle(tint)
            Text(label).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 10).fill(Color.secondary.opacity(0.08)))
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
