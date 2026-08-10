import Foundation

/// Párování iPadu — appka jen zavolá server (`POST /api/pair/usb`), který má
/// usbmuxd přístup a spáruje připojený iPad sám. Žádné CLI.
@MainActor
final class PairService: ObservableObject {
    enum Phase: Equatable {
        case idle
        case running
        case success(udid: String, name: String, address: String?)
        case failed(String)
    }

    @Published private(set) var phase: Phase = .idle

    /// Vypíše UDID zařízení připojených přes USB.
    func listUSBDevices(serverURL: URL) async -> [String] {
        let url = serverURL.appendingPathComponent("api/pair/usb")
        var req = URLRequest(url: url)
        req.timeoutInterval = 8
        guard let (data, resp) = try? await URLSession.shared.data(for: req),
              let http = resp as? HTTPURLResponse, http.statusCode == 200,
              let list = try? JSONDecoder().decode([String].self, from: data)
        else { return [] }
        return list
    }

    /// Spáruje připojený iPad přes server a nechá server zjistit IP (Bonjour).
    func pair(serverURL: URL, address: String?) async {
        phase = .running

        let url = serverURL.appendingPathComponent("api/pair/usb")
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        req.timeoutInterval = 120   // párování čeká na „Trust" na iPadu
        var body: [String: Any] = [:]
        if let address, !address.isEmpty { body["address"] = address }
        req.httpBody = try? JSONSerialization.data(withJSONObject: body)

        do {
            let (data, resp) = try await URLSession.shared.data(for: req)
            guard let http = resp as? HTTPURLResponse else {
                phase = .failed("Neplatná odpověď serveru")
                return
            }
            let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            if (200..<300).contains(http.statusCode) {
                let udid = obj?["udid"] as? String ?? ""
                let name = obj?["name"] as? String ?? "iPad"
                let addr = obj?["address"] as? String
                phase = .success(udid: udid, name: name, address: addr)
            } else {
                phase = .failed(obj?["error"] as? String ?? "Párování selhalo (HTTP \(http.statusCode))")
            }
        } catch {
            phase = .failed(error.localizedDescription)
        }
    }

    func reset() {
        phase = .idle
    }
}
