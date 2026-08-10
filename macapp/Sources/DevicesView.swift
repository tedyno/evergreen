import SwiftUI

struct DevicesView: View {
    @EnvironmentObject var state: AppState
    @EnvironmentObject var pair: PairService
    @State private var showPairSheet = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                HStack {
                    Text(state.t("Spárovaná zařízení", "Paired devices")).font(.headline)
                    Spacer()
                    Button {
                        pair.reset()
                        showPairSheet = true
                    } label: {
                        Label(state.t("Spárovat iPad", "Pair iPad"), systemImage: "cable.connector")
                    }
                }

                if state.devices.isEmpty {
                    ContentUnavailableView(state.t("Žádná zařízení", "No devices"),
                                           systemImage: "ipad.and.iphone.slash",
                                           description: Text(state.t("Připoj iPad USB kabelem a klikni na „Spárovat iPad“.", "Connect an iPad via USB cable and click “Pair iPad”.")))
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
        .navigationTitle(state.t("Zařízení", "Devices"))
        .sheet(isPresented: $showPairSheet) {
            PairSheet()
                .environmentObject(state)
                .environmentObject(pair)
        }
    }
}

/// Pairing wizard — the app calls the server, which pairs the iPad and detects the IP itself.
struct PairSheet: View {
    @EnvironmentObject var state: AppState
    @EnvironmentObject var pair: PairService
    @Environment(\.dismiss) private var dismiss

    @State private var manualIP = ""
    @State private var manualSaved = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text(state.t("Spárovat iPad přes USB", "Pair iPad over USB"))
                .font(.headline)

            VStack(alignment: .leading, spacing: 6) {
                Label(state.t("Připoj iPad USB kabelem a odemkni ho.", "Connect the iPad via USB cable and unlock it."), systemImage: "1.circle")
                Label(state.t("Klikni na Spárovat — na iPadu potvrď „Trust / Důvěřovat“ a zadej kód.", "Click Pair — on the iPad confirm “Trust” and enter the passcode."), systemImage: "2.circle")
                Label(state.t("IP adresu iPadu zjistí Evergreen sám.", "Evergreen detects the iPad’s IP address by itself."), systemImage: "wifi")
                Label(state.t("Pro bezdrátovou instalaci potvrď „Trust“ i podruhé (RemotePairing).", "For wireless install, confirm “Trust” a second time too (RemotePairing)."), systemImage: "antenna.radiowaves.left.and.right")
            }
            .font(.callout)
            .foregroundStyle(.secondary)

            statusArea

            HStack {
                Spacer()
                Button(state.t("Zavřít", "Close")) {
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
                        Text(state.t("Spárovat", "Pair"))
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
            Label(state.t("Páruji… potvrď „Trust“ na iPadu.", "Pairing… confirm “Trust” on the iPad."), systemImage: "hourglass")
                .foregroundStyle(.secondary)
        case .success(let udid, let name, let addr, let wireless):
            VStack(alignment: .leading, spacing: 8) {
                Label(state.t("Spárováno: \(name)", "Paired: \(name)"), systemImage: "checkmark.circle.fill")
                    .foregroundStyle(.green)
                if wireless {
                    Label(state.t("Bezdrátová instalace připravena (RemotePairing).", "Wireless install ready (RemotePairing)."), systemImage: "wifi")
                        .font(.caption).foregroundStyle(.green)
                } else {
                    Label(state.t("Bezdrátovou instalaci se nepodařilo povolit — funguje instalace přes USB.", "Wireless install couldn’t be enabled — USB install works."), systemImage: "wifi.slash")
                        .font(.caption).foregroundStyle(.orange)
                }
                if let addr {
                    Text(state.t("IP zjištěna automaticky: \(addr)", "IP detected automatically: \(addr)"))
                        .font(.caption).foregroundStyle(.secondary)
                } else if manualSaved {
                    Text(state.t("IP uložena ručně.", "IP saved manually."))
                        .font(.caption).foregroundStyle(.secondary)
                } else {
                    // Fallback only when auto-detection fails.
                    Text(state.t("IP se nepodařilo zjistit automaticky (iPad možná ještě není na Wi-Fi). Můžeš ji zadat ručně:", "IP could not be detected automatically (the iPad may not be on Wi-Fi yet). You can enter it manually:"))
                        .font(.caption).foregroundStyle(.orange)
                    HStack {
                        TextField(state.t("např. 10.0.1.7", "e.g. 10.0.1.7"), text: $manualIP)
                            .textFieldStyle(.roundedBorder)
                            .frame(maxWidth: 180)
                        Button(state.t("Uložit IP", "Save IP")) {
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
                    Text(device.address ?? state.t("IP neznámá", "IP unknown"))
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
                    else { Text(state.t("Zjistit IP", "Detect IP")) }
                }
                .disabled(detecting)
            }

            Button(role: .destructive) {
                confirmDelete = true
            } label: {
                Text(state.t("Odebrat", "Remove"))
            }
        }
        .padding(.vertical, 4)
        .confirmationDialog(state.t("Odebrat zařízení \(device.name)?", "Remove device \(device.name)?"), isPresented: $confirmDelete) {
            Button(state.t("Odebrat", "Remove"), role: .destructive) {
                Task { await state.deleteDevice(device) }
            }
            Button(state.t("Zrušit", "Cancel"), role: .cancel) {}
        }
    }
}
