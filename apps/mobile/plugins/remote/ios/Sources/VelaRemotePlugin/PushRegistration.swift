import Foundation
import UIKit
import Security

struct PushFailure: LocalizedError {
    let code: String
    var errorDescription: String? { code }
    init(_ code: String) { self.code = code }
}

private final class PushHTTP: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse, newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) { completionHandler(nil) }
}

@MainActor private final class ApplePushToken {
    static let shared = ApplePushToken()
    private var waiters: [UUID: CheckedContinuation<String, Error>] = [:]
    func obtain() async throws -> String {
        let id = UUID()
        return try await withCheckedThrowingContinuation { continuation in
            waiters[id] = continuation
            UIApplication.shared.registerForRemoteNotifications()
            DispatchQueue.main.asyncAfter(deadline: .now() + 20) {
                self.waiters.removeValue(forKey: id)?.resume(throwing: PushFailure("PUSH_REGISTRATION_FAILED"))
            }
        }
    }
    func resolve(_ result: Result<String, Error>) {
        let pending = waiters; waiters.removeAll()
        for continuation in pending.values { continuation.resume(with: result) }
    }
}

/// Owner credentials and device tokens stay in Keychain. Remote pages receive only a revocable
/// capability for their own connection, never the installation credential or APNs token.
actor PushRegistration {
    static let shared = PushRegistration()
    private let vault = Vault(account: "push")
    private let delegate = PushHTTP()
    private lazy var http = URLSession(configuration: .ephemeral, delegate: delegate, delegateQueue: nil)
    private var lastError: String?
    private var environment: String? {
        let value = Bundle.main.object(forInfoDictionaryKey: "VelaPushEnvironment") as? String
        return ["development", "production"].contains(value ?? "") ? value : nil
    }
    static func registered(_ data: Data) {
        let token = data.map { String(format: "%02x", $0) }.joined()
        Task { @MainActor in ApplePushToken.shared.resolve(.success(token)) }
        Task { await shared.tokenChanged(token) }
    }
    static func failed() {
        Task { @MainActor in ApplePushToken.shared.resolve(.failure(PushFailure("PUSH_REGISTRATION_FAILED"))) }
    }
    private func owner() throws -> String {
        try vault.update { store in
            if let value = store["owner"] as? String { return value }
            var bytes = [UInt8](repeating: 0, count: 32)
            guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else { throw PushFailure("PUSH_STORAGE_FAILED") }
            let value = Data(bytes).base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_").replacingOccurrences(of: "=", with: "")
            store["owner"] = value; return value
        }
    }
    private func request(_ path: String, method: String = "POST", body: [String: Any]? = nil, account: String? = nil) async throws -> [String: Any] {
        var request = URLRequest(url: URL(string: "https://velaterm.com/api/mobile-push" + path)!)
        request.httpMethod = method; request.timeoutInterval = 20
        request.setValue("Bearer \(try owner())", forHTTPHeaderField: "Authorization")
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        if let account { request.setValue(account, forHTTPHeaderField: "X-Vela-Account-Token") }
        if let body { request.httpBody = try JSONSerialization.data(withJSONObject: body) }
        do {
            let (data, response) = try await http.data(for: request)
            guard let response = response as? HTTPURLResponse, (200..<300).contains(response.statusCode), data.count <= 16384 else { throw PushFailure("PUSH_RELAY_UNAVAILABLE") }
            if data.isEmpty { return [:] }
            return try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
        } catch { throw PushFailure("PUSH_RELAY_UNAVAILABLE") }
    }
    func status() async -> [String: Any] {
        let permission = await withCheckedContinuation { continuation in TaskNotifications.shared.permission(request: false) { continuation.resume(returning: $0) } }
        let saved = (try? vault.read()) ?? [:]
        let subscriptions = saved["subscriptions"] as? [String: [String: Any]] ?? [:]
        var result: [String: Any] = ["permission": permission, "configured": environment != nil,
            "enabled": environment != nil && saved["enabled"] as? Bool == true && saved["registeredToken"] as? String != nil && permission == "granted",
            "connections": subscriptions.filter { $0.value["bound"] as? Bool == true }.map { $0.key }]
        if let lastError { result["error"] = lastError }
        return result
    }
    func enable() async throws {
        guard environment != nil else { throw PushFailure("PUSH_NOT_CONFIGURED") }
        let permission = await withCheckedContinuation { continuation in TaskNotifications.shared.permission(request: true) { continuation.resume(returning: $0) } }
        guard permission == "granted" else { throw PushFailure("PUSH_DENIED") }
        let token = try await ApplePushToken.shared.obtain()
        try vault.update { $0["enabled"] = true; $0["token"] = token }
        do { try await register(token); lastError = nil }
        catch { lastError = "PUSH_RELAY_UNAVAILABLE"; throw error }
    }
    private func register(_ token: String) async throws {
        guard let environment else { throw PushFailure("PUSH_NOT_CONFIGURED") }
        let locale = Locale.preferredLanguages.first ?? "en"
        _ = try await request("/installations", body: ["platform": "apns", "environment": environment, "token": token, "locale": locale])
        try vault.update { $0["registeredToken"] = token; $0["registeredAt"] = Date().timeIntervalSince1970 }
    }
    private func tokenChanged(_ token: String) async {
        do {
            let saved = try vault.read()
            try vault.update { $0["token"] = token }
            if saved["enabled"] as? Bool == true && saved["registeredToken"] as? String != token { try await register(token) }
        } catch { lastError = "PUSH_RELAY_UNAVAILABLE" }
    }
    func resume() async {
        guard let saved = try? vault.read(), saved["enabled"] as? Bool == true else { return }
        do {
            let permission = await withCheckedContinuation { continuation in TaskNotifications.shared.permission(request: false) { continuation.resume(returning: $0) } }
            guard permission == "granted" else { try await disable(); return }
            let token = try await ApplePushToken.shared.obtain()
            if saved["registeredToken"] as? String != token || Date().timeIntervalSince1970 - (saved["registeredAt"] as? Double ?? 0) > 86400 { try await register(token) }
        } catch { lastError = (error as? PushFailure)?.code ?? "PUSH_RELAY_UNAVAILABLE" }
    }
    func subscription(_ connection: String) async throws -> [String: Any] {
        let saved = try vault.read()
        guard saved["enabled"] as? Bool == true, saved["registeredToken"] as? String != nil else { throw PushFailure("PUSH_DISABLED") }
        let account = connection.hasPrefix("account_") ? try Vault().read()["accountToken"] as? String : nil
        let result = try await request("/subscriptions", body: ["connectionId": connection], account: account)
        guard let id = result["subscriptionId"] as? String, UUID(uuidString: id) != nil,
              let token = result["publisherToken"] as? String, token.range(of: "^[A-Za-z0-9_-]{43}$", options: .regularExpression) != nil else { throw PushFailure("PUSH_RELAY_UNAVAILABLE") }
        try vault.update { store in
            var subscriptions = store["subscriptions"] as? [String: [String: Any]] ?? [:]
            let bound = subscriptions[connection]?["subscriptionId"] as? String == id && subscriptions[connection]?["bound"] as? Bool == true
            subscriptions[connection] = ["subscriptionId": id, "publisherToken": token, "bound": bound]
            store["subscriptions"] = subscriptions
        }
        return ["subscriptionId": id, "publisherToken": token]
    }
    func bound(_ connection: String, active: Bool) throws {
        try vault.update { store in
            var subscriptions = store["subscriptions"] as? [String: [String: Any]] ?? [:]
            guard subscriptions[connection] != nil else { return }
            // A failed reconnect does not cancel a host subscription that is already delivering.
            // Revocation and connection edits remove the entry explicitly.
            let acknowledged = subscriptions[connection]?["bound"] as? Bool == true
            subscriptions[connection]?["bound"] = active || acknowledged
            store["subscriptions"] = subscriptions
        }
        if !active { lastError = "PUSH_HOST_UNAVAILABLE" } else { lastError = nil }
    }
    func isBound(_ connection: String) -> Bool {
        guard environment != nil, let saved = try? vault.read(), saved["enabled"] as? Bool == true else { return false }
        return (saved["subscriptions"] as? [String: [String: Any]])?[connection]?["bound"] as? Bool == true
    }
    func revoke(_ connection: String) async throws {
        guard let subscriptions = try vault.read()["subscriptions"] as? [String: [String: Any]], let id = subscriptions[connection]?["subscriptionId"] as? String else { return }
        _ = try await request("/subscriptions/" + id, method: "DELETE")
        try vault.update { store in
            var current = store["subscriptions"] as? [String: [String: Any]] ?? [:]
            current.removeValue(forKey: connection); store["subscriptions"] = current
        }
    }
    func disable() async throws {
        let saved = try vault.read()
        if saved["registeredToken"] != nil { _ = try await request("/installation", method: "DELETE") }
        try vault.update { $0["enabled"] = false; $0.removeValue(forKey: "registeredToken"); $0["subscriptions"] = [String: [String: Any]]() }
        lastError = nil
    }
    func test(_ connection: String) async throws {
        guard let subscriptions = try vault.read()["subscriptions"] as? [String: [String: Any]], let id = subscriptions[connection]?["subscriptionId"] as? String else { throw PushFailure("PUSH_HOST_UNAVAILABLE") }
        _ = try await request("/subscriptions/\(id)/test", body: [:])
    }
}
