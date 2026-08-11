import SwiftUI
import UniformTypeIdentifiers

struct AppsView: View {
    @EnvironmentObject var state: AppState
    @State private var isTargeted = false
    @State private var errorMessage: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                // We only show the account's App ID overview after signing in with an Apple ID —
                // only then do we know the actual account state (see Developer Services).
                if state.account?.authState == "logged_in" {
                    FreeAccountCard()
                }

                if state.status?.wireless == false {
                    Label(state.t("Bezdrátová (Wi-Fi) instalace vyžaduje nainstalovaný Xcode. Bez něj instaluj přes USB kabel.",
                                  "Wireless (Wi-Fi) install requires Xcode to be installed. Without it, install over a USB cable."),
                          systemImage: "exclamationmark.triangle.fill")
                        .font(.callout)
                        .foregroundStyle(.orange)
                        .padding(10)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 10).fill(Color.orange.opacity(0.12)))
                }

                dropZone

                if let p = state.uploadProgress {
                    ProgressView(value: p) {
                        Text(state.t("Nahrávám… \(Int(p * 100)) %", "Uploading… \(Int(p * 100)) %"))
                            .font(.caption)
                    }
                }

                Text(state.t("Katalog", "Catalog"))
                    .font(.headline)

                if state.ipas.isEmpty {
                    ContentUnavailableView(state.t("Zatím žádné IPA", "No IPAs yet"),
                                           systemImage: "shippingbox",
                                           description: Text(state.t("Přetáhni sem .ipa soubor nebo klikni na plochu výše.", "Drag an .ipa file here or click the area above.")))
                        .frame(maxWidth: .infinity, minHeight: 160)
                } else {
                    ForEach(state.ipas) { ipa in
                        IpaRow(ipa: ipa)
                        Divider()
                    }
                }
            }
            .padding(20)
        }
        .navigationTitle(state.t("Aplikace", "Apps"))
        .alert(state.t("Chyba", "Error"), isPresented: Binding(get: { errorMessage != nil }, set: { if !$0 { errorMessage = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(errorMessage ?? "")
        }
    }

    private var dropZone: some View {
        Button {
            pickFile()
        } label: {
            VStack(spacing: 8) {
                Image(systemName: "arrow.down.doc")
                    .font(.system(size: 28))
                    .foregroundStyle(.secondary)
                Text(state.t("Přetáhni sem .ipa nebo klikni pro výběr", "Drag an .ipa here or click to select"))
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, minHeight: 120)
            .background(
                RoundedRectangle(cornerRadius: 12)
                    .strokeBorder(style: StrokeStyle(lineWidth: 2, dash: [6]))
                    .foregroundStyle(isTargeted ? Color.accentColor : Color.secondary.opacity(0.4))
            )
        }
        .buttonStyle(.plain)
        .onDrop(of: [.fileURL], isTargeted: $isTargeted) { providers in
            handleDrop(providers)
        }
    }

    private func pickFile() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        if let ipaType = UTType(filenameExtension: "ipa") {
            panel.allowedContentTypes = [ipaType]
        }
        if panel.runModal() == .OK, let url = panel.url {
            upload(url)
        }
    }

    private func handleDrop(_ providers: [NSItemProvider]) -> Bool {
        guard let provider = providers.first else { return false }
        _ = provider.loadObject(ofClass: URL.self) { url, _ in
            guard let url, url.pathExtension.lowercased() == "ipa" else { return }
            Task { @MainActor in upload(url) }
        }
        return true
    }

    private func upload(_ url: URL) {
        Task {
            do {
                try await state.uploadIpa(fileURL: url)
            } catch {
                errorMessage = (error as? ApiError)?.message ?? error.localizedDescription
            }
        }
    }
}

/// Account App ID overview based on the actual state on Apple (Developer Services).
struct FreeAccountCard: View {
    @EnvironmentObject var state: AppState

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Label(state.t("App ID na účtu", "App IDs on account"), systemImage: "person.badge.key")
                    .font(.headline)
                Spacer()
                if state.appIdLoading && state.appIdInfo == nil {
                    ProgressView().controlSize(.small)
                } else {
                    if let info = state.appIdInfo {
                        if info.isPaid {
                            Text("\(info.count)").font(.title3.monospacedDigit())
                        } else {
                            Text("\(info.count)/\(info.max)")
                                .font(.title3.monospacedDigit())
                                .foregroundStyle(info.count >= info.max ? .red : .primary)
                        }
                    }
                    Button {
                        Task { await state.refreshAppIds(force: true) }
                    } label: {
                        Image(systemName: "arrow.clockwise")
                    }
                    .buttonStyle(.borderless)
                    .controlSize(.small)
                    .help(state.t("Obnovit z Apple", "Refresh from Apple"))
                }
            }

            if let info = state.appIdInfo {
                if info.isPaid {
                    Label(state.t("Placený Apple Developer účet", "Paid Apple Developer account"),
                          systemImage: "checkmark.seal.fill")
                        .font(.caption).foregroundStyle(.green)
                    Text(state.t("Bez limitu App ID, profily platí ~1 rok — obnova stačí ~jednou ročně. Team \(info.teamId).", "No App ID limit, ~1-year profiles — renews about once a year. Team \(info.teamId)."))
                        .font(.caption2).foregroundStyle(.secondary)
                } else {
                    HStack(spacing: 4) {
                        ForEach(0..<info.max, id: \.self) { i in
                            RoundedRectangle(cornerRadius: 3)
                                .fill(i < info.count ? Color.accentColor : Color.secondary.opacity(0.2))
                                .frame(height: 7)
                        }
                    }
                    Text(state.t("Skutečný stav účtu (i App ID z AltStoru/Xcode). Free limit: 10 App ID / 7 dní, 3 aktivní appky na zařízení, profil 7 dní. Team \(info.teamId).", "Actual account state (including App IDs from AltStore/Xcode). Free limit: 10 App IDs / 7 days, 3 active apps per device, 7-day profile. Team \(info.teamId)."))
                        .font(.caption2).foregroundStyle(.secondary)
                }

                if !info.appIds.isEmpty {
                    Divider()
                    ForEach(info.appIds) { a in
                        VStack(alignment: .leading, spacing: 1) {
                            Text(a.name).font(.caption)
                            Text(a.identifier).font(.caption2).foregroundStyle(.secondary).lineLimit(1)
                        }
                    }
                }
            }
        }
        .padding(14)
        .background(RoundedRectangle(cornerRadius: 12).fill(Color.secondary.opacity(0.08)))
        // Load the account's App ID state automatically (cheap after the token is cached).
        .task { await state.refreshAppIds() }
    }
}

struct IpaRow: View {
    @EnvironmentObject var state: AppState
    let ipa: Ipa

    @State private var selectedDevice: String = ""
    @State private var installError: String?

    var body: some View {
        HStack(spacing: 12) {
            AsyncImage(url: state.iconURLSync(for: ipa)) { phase in
                if let image = phase.image {
                    image.resizable().aspectRatio(contentMode: .fit)
                } else {
                    RoundedRectangle(cornerRadius: 9)
                        .fill(Color.secondary.opacity(0.2))
                        .overlay(Image(systemName: "app").foregroundStyle(.secondary))
                }
            }
            .frame(width: 44, height: 44)
            .clipShape(RoundedRectangle(cornerRadius: 9))

            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(ipa.name).fontWeight(.medium)
                    if let v = ipa.version {
                        Text(v).font(.caption).foregroundStyle(.secondary)
                    }
                }
                Text(ipa.bundleId)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }

            Spacer()

            Picker("", selection: $selectedDevice) {
                ForEach(state.devices) { d in
                    Text(d.name).tag(d.udid)
                }
            }
            .labelsHidden()
            .frame(maxWidth: 180)
            .disabled(state.devices.isEmpty)

            Button(state.t("Instalovat", "Install")) {
                install()
            }
            .disabled(state.devices.isEmpty || selectedDevice.isEmpty)

            Button(role: .destructive) {
                Task { await state.deleteIpa(ipa) }
            } label: {
                Image(systemName: "trash")
            }
            .buttonStyle(.borderless)
        }
        .padding(.vertical, 4)
        .onAppear {
            if selectedDevice.isEmpty { selectedDevice = state.devices.first?.udid ?? "" }
        }
        .alert(state.t("Instalace selhala", "Installation failed"), isPresented: Binding(get: { installError != nil }, set: { if !$0 { installError = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(installError ?? "")
        }
    }

    private func install() {
        Task {
            do {
                try await state.install(ipa: ipa, onDevice: selectedDevice)
            } catch {
                installError = (error as? ApiError)?.message ?? error.localizedDescription
            }
        }
    }
}
