import Foundation
import UserNotifications

/// Local fallback and APNs delivery share one delegate and one connection/session navigation path.
public final class TaskNotifications: NSObject, UNUserNotificationCenterDelegate {
    public static let shared = TaskNotifications()
    private var pending: [String: Any]?
    private var opened: [String] = []
    var onOpen: (([String: Any]) -> Void)? {
        didSet { if let pending, let onOpen { self.pending = nil; onOpen(pending) } }
    }
    public func activate() { UNUserNotificationCenter.current().delegate = self }
    public func registeredForPush(_ data: Data) { PushRegistration.registered(data) }
    public func pushRegistrationFailed() { PushRegistration.failed() }
    public func resumePush() { Task { await PushRegistration.shared.resume() } }
    func permission(request: Bool, completion: @escaping (String) -> Void) {
        let center = UNUserNotificationCenter.current()
        center.getNotificationSettings { settings in
            if request && settings.authorizationStatus == .notDetermined {
                center.requestAuthorization(options: [.alert, .sound, .badge]) { allowed, _ in completion(allowed ? "granted" : "denied") }
                return
            }
            switch settings.authorizationStatus {
            case .authorized, .provisional, .ephemeral: completion("granted")
            case .notDetermined: completion("default")
            default: completion("denied")
            }
        }
    }
    func send(connectionID: String, body: [String: Any], completion: @escaping (String?) -> Void) {
        guard let title = body["title"] as? String, !title.isEmpty, title.utf8.count <= 1024,
              let text = body["body"] as? String, text.utf8.count <= 4096 else { completion("Invalid notification"); return }
        let session = body["sessionId"] as? String ?? ""
        guard session.utf8.count <= 256 else { completion("Invalid session"); return }
        permission(request: false) { permission in
            guard permission == "granted" else { completion("Notifications are not permitted"); return }
            let content = UNMutableNotificationContent()
            content.title = title; content.body = text
            if body["sound"] as? Bool == true { content.sound = .default }
            content.userInfo = ["id": connectionID, "sessionId": session]
            content.threadIdentifier = connectionID
            let request = UNNotificationRequest(identifier: connectionID + ":" + session, content: content, trigger: nil)
            UNUserNotificationCenter.current().add(request) { error in completion(error == nil ? nil : "Could not deliver notification") }
        }
    }
    public func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent notification: UNNotification, withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void) {
        if let event = notification.request.content.userInfo["eventId"] as? String {
            var seen = UserDefaults.standard.stringArray(forKey: "vela.push.presented") ?? []
            if seen.contains(event) { completionHandler([]); return }
            seen.append(event); UserDefaults.standard.set(Array(seen.suffix(200)), forKey: "vela.push.presented")
        }
        completionHandler([.banner, .list, .sound])
    }
    public func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse, withCompletionHandler completionHandler: @escaping () -> Void) {
        open(response)
        completionHandler()
    }
    /// Scene cold-start delivery and the notification delegate may report the same tap.
    public func open(_ response: UNNotificationResponse) {
        if response.actionIdentifier == UNNotificationDefaultActionIdentifier,
           let id = response.notification.request.content.userInfo["id"] as? String, !id.isEmpty, id.utf8.count <= 256 {
            let event: [String: Any] = ["id": id, "sessionId": response.notification.request.content.userInfo["sessionId"] as? String ?? ""]
            let key = response.notification.request.content.userInfo["eventId"] as? String
                ?? response.notification.request.identifier + ":" + String(response.notification.date.timeIntervalSince1970)
            DispatchQueue.main.async {
                guard !self.opened.contains(key) else { return }
                self.opened.append(key); self.opened = Array(self.opened.suffix(200))
                if let onOpen = self.onOpen { onOpen(event) } else { self.pending = event }
            }
        }
    }
}
