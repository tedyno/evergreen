import SwiftUI

struct AccountView: View {
    @EnvironmentObject var state: AppState

    @State private var appleId = ""
    @State private var password = ""
    @State private var code = ""
    @State private var busy = false
    @State private var errorMessage: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text("Apple ID")
                    .font(.headline)

                content
            }
            .frame(maxWidth: 420, alignment: .leading)
            .padding(20)
        }
        .navigationTitle("Účet")
        .alert("Chyba", isPresented: Binding(get: { errorMessage != nil }, set: { if !$0 { errorMessage = nil } })) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(errorMessage ?? "")
        }
    }

    @ViewBuilder
    private var content: some View {
        let acc = state.account
        if acc?.authState == "logged_in" {
            loggedIn(acc)
        } else if acc?.authState == "needs_2fa" {
            twoFactor
        } else {
            loginForm
        }
    }

    private func loggedIn(_ acc: Account?) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Label {
                Text("Přihlášeno jako ") + Text(acc?.appleId ?? "").bold()
            } icon: {
                Image(systemName: "checkmark.seal.fill").foregroundStyle(.green)
            }
            if let team = acc?.teamId {
                Text("Tým: \(team)").font(.caption).foregroundStyle(.secondary)
            }
            Button("Odhlásit", role: .destructive) {
                Task { await state.logout() }
            }
        }
    }

    private var twoFactor: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Zadej 2FA kód z důvěryhodného zařízení:")
                .foregroundStyle(.secondary)
            TextField("123456", text: $code)
                .textFieldStyle(.roundedBorder)
                .frame(maxWidth: 160)
            Button("Ověřit") { verify() }
                .disabled(busy || code.isEmpty)
        }
    }

    private var loginForm: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField("mail@icloud.com", text: $appleId)
                .textFieldStyle(.roundedBorder)
            SecureField("Heslo", text: $password)
                .textFieldStyle(.roundedBorder)
            Button("Přihlásit") { login() }
                .disabled(busy || appleId.isEmpty || password.isEmpty)
            Text("Heslo se ukládá na serveru šifrovaně (AES-256-GCM) a slouží jen k podpisu.")
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private func login() {
        busy = true
        Task {
            defer { busy = false }
            do {
                _ = try await state.login(appleId: appleId, password: password)
                password = ""
            } catch {
                errorMessage = (error as? ApiError)?.message ?? error.localizedDescription
            }
        }
    }

    private func verify() {
        busy = true
        Task {
            defer { busy = false }
            do {
                _ = try await state.submit2FA(code: code)
                code = ""
            } catch {
                errorMessage = (error as? ApiError)?.message ?? error.localizedDescription
            }
        }
    }
}

struct SettingsView: View {
    @EnvironmentObject var state: AppState
    @EnvironmentObject var server: ServerController
    @State private var draft = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Server")
                .font(.headline)

            Toggle("Spouštět vlastní server v této appce", isOn: Binding(
                get: { state.useLocalServer },
                set: { newValue in
                    Task {
                        state.useLocalServer = newValue
                        if newValue {
                            await server.startIfNeeded()
                            await state.activate(baseURL: server.localBaseURL)
                        } else {
                            server.stop()
                            await state.switchToRemote()
                        }
                    }
                }
            ))

            if state.useLocalServer {
                HStack(spacing: 8) {
                    serverStateIndicator
                    Text(serverStateText).font(.caption).foregroundStyle(.secondary)
                }
                Text("Server běží jako podproces na 127.0.0.1:\(server.port), data v Application Support. Anisette jede nativně (AOSKit), žádný Docker.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                TextField("Adresa vzdáleného serveru", text: $draft)
                    .textFieldStyle(.roundedBorder)
                    .frame(minWidth: 260)
                Text("Např. http://10.0.1.3:8080 — server běžící jinde (NAS/RPi).")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Button("Uložit a připojit") {
                    Task {
                        state.remoteServerURLString = draft
                        await state.switchToRemote()
                    }
                }
            }
        }
        .padding(20)
        .frame(width: 380)
        .onAppear { draft = state.remoteServerURLString }
    }

    @ViewBuilder
    private var serverStateIndicator: some View {
        let color: Color = {
            switch server.state {
            case .running: return .green
            case .starting: return .orange
            case .stopped: return .secondary
            case .failed: return .red
            }
        }()
        Circle().fill(color).frame(width: 8, height: 8)
    }

    private var serverStateText: String {
        switch server.state {
        case .running: return "Server běží"
        case .starting: return "Spouští se…"
        case .stopped: return "Zastaven"
        case .failed(let msg): return "Chyba: \(msg)"
        }
    }
}
