import SwiftUI

/// Nastavení: adresa serveru + stav Apple ID účtu (jen náhled; přihlášení se
/// dělá ve web UI serveru, kde se bezpečně zadává heslo a 2FA).
struct SettingsView: View {
    @EnvironmentObject var api: API
    @State private var account: AccountStatus?
    @State private var devices: [DeviceItem] = []
    @State private var draftURL: String = ""

    var body: some View {
        NavigationStack {
            Form {
                Section("Server") {
                    TextField("http://homesign.local:8080", text: $draftURL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.URL)
                    Button("Uložit") { api.serverURL = draftURL }
                        .disabled(draftURL.isEmpty || draftURL == api.serverURL)
                }
                Section("Apple ID") {
                    if let a = account {
                        LabeledContent("Stav", value: a.auth_state)
                        if let id = a.apple_id { LabeledContent("Účet", value: id) }
                        if let t = a.team_id { LabeledContent("Tým", value: t) }
                    } else {
                        Text("Nedostupné").foregroundStyle(.secondary)
                    }
                    Text("Přihlášení a 2FA se provádí ve web UI serveru.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Section("Zařízení") {
                    if devices.isEmpty {
                        Text("Žádná spárovaná zařízení").foregroundStyle(.secondary)
                    }
                    ForEach(devices) { d in
                        VStack(alignment: .leading) {
                            Text(d.name)
                            Text("\(d.address ?? "IP ?") · iOS \(d.ios_version ?? "?")")
                                .font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            .navigationTitle("Nastavení")
            .task {
                draftURL = api.serverURL
                account = try? await api.account()
                devices = (try? await api.devices()) ?? []
            }
        }
    }
}
