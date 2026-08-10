import SwiftUI

struct DevicesView: View {
    @EnvironmentObject var state: AppState
    @EnvironmentObject var pair: PairService
    @State private var showPairSheet = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text("Spárovaná zařízení").font(.headline)
                    Spacer()
                    Button {
                        pair.reset()
                        showPairSheet = true
                    } label: {
                        Label("Spárovat iPad", systemImage: "cable.connector")
                    }
                }

                if state.devices.isEmpty {
                    ContentUnavailableView("Žádná zařízení",
                                           systemImage: "ipad.and.iphone.slash",
                                           description: Text("Připoj iPad USB kabelem a klikni na „Spárovat iPad“."))
                        .frame(maxWidth: .infinity, minHeight: 160)
                } else {
                    ForEach(state.devices) { device in
                        DeviceRow(device: device)
                        Divider()
                    }
                }
            }
            .padding(20)
        }
        .navigationTitle("Zařízení")
        .sheet(isPresented: $showPairSheet) {
            PairSheet()
                .environmentObject(state)
                .environmentObject(pair)
        }
    }
}

/// Průvodce párováním — appka zavolá server, ten spáruje iPad a IP zjistí sám.
struct PairSheet: View {
    @EnvironmentObject var state: AppState
    @EnvironmentObject var pair: PairService
    @Environment(\.dismiss) private var dismiss

    @State private var manualIP = ""
    @State private var manualSaved = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Spárovat iPad přes USB")
                .font(.headline)

            VStack(alignment: .leading, spacing: 6) {
                Label("Připoj iPad USB kabelem a odemkni ho.", systemImage: "1.circle")
                Label("Klikni na Spárovat — na iPadu potvrď „Trust / Důvěřovat“ a zadej kód.", systemImage: "2.circle")
                Label("IP adresu iPadu zjistí homesign sám.", systemImage: "wifi")
            }
            .font(.callout)
            .foregroundStyle(.secondary)

            statusArea

            HStack {
                Spacer()
                Button("Zavřít") {
                    Task { await state.refreshDevices() }
                    dismiss()
                }
                Button {
                    Task {
                        await pair.pair(serverURL: state.baseURL, address: nil)
                        await state.refreshDevices()
                    }
                } label: {
                    if pair.phase == .running {
                        ProgressView().controlSize(.small)
                    } else {
                        Text("Spárovat")
                    }
                }
                .keyboardShortcut(.defaultAction)
                .disabled(pair.phase == .running)
            }
        }
        .padding(20)
        .frame(width: 460)
    }

    @ViewBuilder
    private var statusArea: some View {
        switch pair.phase {
        case .idle:
            EmptyView()
        case .running:
            Label("Páruji… potvrď „Trust“ na iPadu.", systemImage: "hourglass")
                .foregroundStyle(.secondary)
        case .success(let udid, let name, let addr):
            VStack(alignment: .leading, spacing: 8) {
                Label("Spárováno: \(name)", systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                if let addr {
                    Text("IP zjištěna automaticky: \(addr)")
                        .font(.caption).foregroundStyle(.secondary)
                } else if manualSaved {
                    Text("IP uložena ručně.")
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    // Fallback jen když se auto-detekce nepovede.
                    Text("IP se nepodařilo zjistit automaticky (iPad možná ještě není na Wi-Fi). Můžeš ji zadat ručně:")
                        .font(.caption).foregroundStyle(.orange)
                    HStack {
                        TextField("např. 10.0.1.7", text: $manualIP)
                            .textFieldStyle(.roundedBorder)
                            .frame(maxWidth: 180)
                        Button("Uložit IP") {
                            Task {
                                try? await state.setDeviceAddress(udid: udid, address: manualIP)
                                manualSaved = true
                            }
                        }
                        .disabled(manualIP.isEmpty)
                    }
                }
            }
        case .failed(let msg):
            Label(msg, systemImage: "exclamationmark.triangle.fill")
                .foregroundStyle(.red)
                .font(.callout)
        }
    }
}

struct DeviceRow: View {
    @EnvironmentObject var state: AppState
    let device: Device
    @State private var confirmDelete = false
    @State private var detecting = false

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "ipad")
                .font(.title2)
                .foregroundStyle(.secondary)
                .frame(width: 40)

            VStack(alignment: .leading, spacing: 2) {
                Text(device.name).fontWeight(.medium)
                HStack(spacing: 6) {
                    Text(device.address ?? "IP neznámá")
                        .foregroundStyle(device.address == nil ? .orange : .secondary)
                    Text("·")
                    Text("iOS \(device.iosVersion ?? "?")")
                    Text("·")
                    Text(device.udid).truncationMode(.middle).lineLimit(1)
                }
                .font(.caption)
                .foregroundStyle(.secondary)
            }

            Spacer()

            if device.address == nil {
                Button {
                    detecting = true
                    Task {
                        await state.detectDeviceIP(udid: device.udid)
                        detecting = false
                    }
                } label: {
                    if detecting { ProgressView().controlSize(.small) }
                    else { Text("Zjistit IP") }
                }
                .disabled(detecting)
            }

            Button(role: .destructive) {
                confirmDelete = true
            } label: {
                Text("Odebrat")
            }
        }
        .padding(.vertical, 4)
        .confirmationDialog("Odebrat zařízení \(device.name)?", isPresented: $confirmDelete) {
            Button("Odebrat", role: .destructive) {
                Task { await state.deleteDevice(device) }
            }
            Button("Zrušit", role: .cancel) {}
        }
    }
}
