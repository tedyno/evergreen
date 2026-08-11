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
                Text(state.t("Apple ID", "Apple ID"))
                    .font(.headline)

                content
            }
            .frame(maxWidth: 420, alignment: .leading)
            .padding(20)
        }
        .navigationTitle(state.t("Účet", "Account"))
        .alert(state.t("Chyba", "Error"), isPresented: Binding(get: { errorMessage != nil }, set: { if !$0 { errorMessage = nil } })) {
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
                Text(state.t("Přihlášeno jako ", "Signed in as ")) + Text(acc?.appleId ?? "").bold()
            } icon: {
                Image(systemName: "checkmark.seal.fill").foregroundStyle(.green)
            }
            if let team = acc?.teamId {
                Text(state.t("Tým: \(team)", "Team: \(team)")).font(.caption).foregroundStyle(.secondary)
            }
            Button(state.t("Odhlásit", "Sign out"), role: .destructive) {
                Task { await state.logout() }
            }
        }
    }

    private var twoFactor: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(state.t("Zadej 2FA kód z důvěryhodného zařízení:", "Enter the 2FA code from a trusted device:"))
                .foregroundStyle(.secondary)
            TextField("123456", text: $code)
                .textFieldStyle(.roundedBorder)
                .frame(maxWidth: 160)
            Button(state.t("Ověřit", "Verify")) { verify() }
                .disabled(busy || code.isEmpty)
        }
    }

    private var loginForm: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField("mail@icloud.com", text: $appleId)
                .textFieldStyle(.roundedBorder)
            SecureField(state.t("Heslo", "Password"), text: $password)
                .textFieldStyle(.roundedBorder)
            Button(state.t("Přihlásit", "Sign in")) { login() }
                .disabled(busy || appleId.isEmpty || password.isEmpty)
            Text(state.t("Heslo se ukládá na serveru šifrovaně (AES-256-GCM) a slouží jen k podpisu.", "The password is stored encrypted on the server (AES-256-GCM) and is used only for signing."))
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
    @AppStorage("showMenuBarIcon") private var showMenuBarIcon = true
    @State private var draft = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(state.t("Vzhled", "Appearance"))
                .font(.headline)
            Picker(state.t("Jazyk", "Language"), selection: $state.appLanguage) {
                Text(state.t("Systém", "System")).tag("system")
                Text("Čeština").tag("cs")
                Text("English").tag("en")
            }
            .frame(maxWidth: 280)
            Toggle(state.t("Zobrazit ikonu v horní liště (menu bar)", "Show the icon in the menu bar"), isOn: $showMenuBarIcon)

            Divider()

            Text(state.t("Server", "Server"))
                .font(.headline)

            Toggle(state.t("Spouštět server na pozadí (běží i po zavření appky)", "Run the server in the background (keeps running after you close the app)"), isOn: Binding(
                get: { state.useLocalServer },
                set: { newValue in
                    Task {
                        state.useLocalServer = newValue
                        if newValue {
                            await server.startIfNeeded()
                            await state.activate(baseURL: server.localBaseURL)
                        } else {
                            server.uninstallAgent()
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
                Text(state.t("Server běží jako LaunchAgent (com.evergreen.server) na 127.0.0.1:\(server.port) — startuje i po přihlášení a drží automatickou obnovu, i když je appka zavřená. Data v Application Support, anisette nativně (AOSKit), žádný Docker.", "The server runs as a LaunchAgent (com.evergreen.server) on 127.0.0.1:\(server.port) — it also starts at login and keeps auto-refresh going even when the app is closed. Data in Application Support, native anisette (AOSKit), no Docker."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                TextField(state.t("Adresa vzdáleného serveru", "Remote server address"), text: $draft)
                    .textFieldStyle(.roundedBorder)
                    .frame(minWidth: 260)
                Text(state.t("Např. http://10.0.1.3:8080 — server běžící jinde (NAS/RPi).", "E.g. http://10.0.1.3:8080 — a server running elsewhere (NAS/RPi)."))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                Button(state.t("Uložit a připojit", "Save and connect")) {
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
        case .running: return state.t("Server běží", "Server running")
        case .starting: return state.t("Spouští se…", "Starting…")
        case .stopped: return state.t("Zastaven", "Stopped")
        case .failed(let msg): return state.t("Chyba: \(msg)", "Error: \(msg)")
        }
    }
}
