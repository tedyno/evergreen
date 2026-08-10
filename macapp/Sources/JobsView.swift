import SwiftUI

struct JobsView: View {
    @EnvironmentObject var state: AppState

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 12) {
                if state.jobs.isEmpty {
                    ContentUnavailableView("Žádné úlohy",
                                           systemImage: "list.bullet.rectangle",
                                           description: Text("Instalace a obnovy se objeví tady."))
                        .frame(maxWidth: .infinity, minHeight: 160)
                } else {
                    ForEach(state.jobs) { job in
                        JobRow(job: job)
                        Divider()
                    }
                }
            }
            .padding(20)
        }
        .navigationTitle("Úlohy")
    }
}

struct JobRow: View {
    @EnvironmentObject var state: AppState
    let job: HSJob

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                if job.isActive {
                    ProgressView().controlSize(.small)
                }
                Text("\(jobKindLabel) #\(job.id)")
                    .fontWeight(.medium)
                statusBadge
                Spacer()
                if job.isActive {
                    Button("Zrušit", role: .destructive) {
                        Task { await state.cancelJob(job.id) }
                    }
                    .buttonStyle(.borderless)
                    .controlSize(.small)
                }
                if let d = job.createdDate {
                    Text(timeString(d))
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            if let msg = job.message, !msg.isEmpty {
                Text(msg)
                    .font(.caption)
                    .foregroundStyle(job.status == "error" ? .red : .secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if job.isActive {
                HStack(spacing: 8) {
                    ProgressView(value: Double(job.progress) / 100.0)
                    Text("\(job.progress) %")
                        .font(.caption2.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(.vertical, 4)
    }

    private var jobKindLabel: String {
        switch job.kind {
        case "install": return "Instalace"
        case "refresh": return "Obnova"
        default: return job.kind
        }
    }

    private func timeString(_ d: Date) -> String {
        let f = DateFormatter()
        f.dateFormat = Calendar.current.isDateInToday(d) ? "HH:mm:ss" : "d.M. HH:mm"
        return f.string(from: d)
    }

    @ViewBuilder
    private var statusBadge: some View {
        let (text, color): (String, Color) = {
            switch job.status {
            case "done": return ("hotovo", .green)
            case "error": return ("chyba", .red)
            case "running": return ("běží", .blue)
            case "queued": return ("ve frontě", .secondary)
            default: return (job.status, .secondary)
            }
        }()
        Text(text)
            .font(.caption2)
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .overlay(Capsule().strokeBorder(color.opacity(0.6)))
            .foregroundStyle(color)
    }
}
