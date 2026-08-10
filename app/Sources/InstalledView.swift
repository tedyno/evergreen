import SwiftUI

/// Přehled instalací a jejich expirací + živý stav běžících úloh.
struct InstalledView: View {
    @EnvironmentObject var api: API
    @State private var installs: [InstallationItem] = []
    @State private var apps: [IpaItem] = []
    @State private var jobs: [JobItem] = []
    @State private var error: String?

    private let timer = Timer.publish(every: 2, on: .main, in: .common).autoconnect()

    var body: some View {
        NavigationStack {
            List {
                if !activeJobs.isEmpty {
                    Section("Probíhá") {
                        ForEach(activeJobs) { job in
                            VStack(alignment: .leading, spacing: 4) {
                                Text("\(job.kind) #\(job.id)").font(.subheadline)
                                ProgressView(value: Double(job.progress) / 100)
                                if let m = job.message {
                                    Text(m).font(.caption).foregroundStyle(.secondary)
                                }
                            }
                        }
                    }
                }
                Section("Nainstalované") {
                    if installs.isEmpty {
                        Text("Zatím nic nainstalováno").foregroundStyle(.secondary)
                    }
                    ForEach(installs) { inst in
                        InstallRow(inst: inst, appName: appName(inst.ipa_id))
                    }
                }
            }
            .navigationTitle("Nainstalované")
            .refreshable { await load() }
            .task { await load() }
            .onReceive(timer) { _ in Task { await loadJobs() } }
        }
    }

    private var activeJobs: [JobItem] {
        jobs.filter { $0.status == "queued" || $0.status == "running" }
    }

    private func appName(_ id: String) -> String {
        apps.first { $0.id == id }?.name ?? id
    }

    private func load() async {
        do {
            async let i = api.installations()
            async let a = api.apps()
            installs = try await i
            apps = try await a
            await loadJobs()
            error = nil
        } catch { self.error = error.localizedDescription }
    }

    private func loadJobs() async {
        jobs = (try? await api.jobs()) ?? jobs
    }
}

private struct InstallRow: View {
    let inst: InstallationItem
    let appName: String

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 2) {
                Text(appName).font(.headline)
                Text(inst.signed_bundle_id).font(.caption).foregroundStyle(.secondary)
                    .lineLimit(1).truncationMode(.middle)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 2) {
                statusBadge
                if let exp = expiryText { Text(exp).font(.caption2).foregroundStyle(.secondary) }
            }
        }
    }

    private var statusBadge: some View {
        Text(inst.status)
            .font(.caption2).bold()
            .padding(.horizontal, 8).padding(.vertical, 2)
            .background(color.opacity(0.2), in: Capsule())
            .foregroundStyle(color)
    }

    private var color: Color {
        switch inst.status {
        case "installed": return .green
        case "error", "expired": return .red
        default: return .orange
        }
    }

    private var expiryText: String? {
        guard let s = inst.profile_expires,
              let date = ISO8601DateFormatter().date(from: s) else { return nil }
        let days = Calendar.current.dateComponents([.day], from: Date(), to: date).day ?? 0
        return days >= 0 ? "vyprší za \(days) d" : "vypršelo"
    }
}
