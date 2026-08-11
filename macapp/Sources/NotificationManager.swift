import Foundation
import UserNotifications

/// Thin wrapper over local (user) notifications — used to surface problems the user
/// needs to act on even when Evergreen's window is closed (resign failed, device
/// unreachable, login needs 2FA, an automatic renewal finished).
@MainActor
final class NotificationManager {
    static let shared = NotificationManager()

    /// Asks for permission once. Safe to call on every launch.
    func requestAuthorization() {
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { _, _ in }
    }

    /// Posts a notification immediately. The system drops it if permission was denied.
    func post(title: String, body: String) {
        let content = UNMutableNotificationContent()
        content.title = title
        content.body = body
        content.sound = .default
        let req = UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil)
        UNUserNotificationCenter.current().add(req)
    }
}
