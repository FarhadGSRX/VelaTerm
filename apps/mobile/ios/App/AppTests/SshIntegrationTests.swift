import XCTest
import Security
import UIKit
import Capacitor
import SafariServices
@testable import VelaRemotePlugin

/// Opt-in native account checks against the public service. Authorization is completed in the dedicated vlx-browser profile.
final class RemoteAccountIntegrationTests: XCTestCase {
    @MainActor private func account(_ plugin: VelaRemotePlugin, _ action: String, deviceId: String? = nil) async throws -> [String: Any] {
        try await withCheckedThrowingContinuation { continuation in
            var options: [String: Any] = ["action": action]
            if let deviceId { options["deviceId"] = deviceId }
            let call = CAPPluginCall(callbackId: UUID().uuidString, methodName: "account", options: options, success: { result, _ in
                continuation.resume(returning: result?.data ?? [:])
            }, error: { error in
                continuation.resume(throwing: NSError(domain: error?.code ?? "account", code: 1, userInfo: [NSLocalizedDescriptionKey: error?.message ?? "Account request failed"]))
            })!
            plugin.account(call)
        }
    }

    @MainActor func testNativeAuthorizationRecoveryAndLogout() async throws {
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let marker = documents.appendingPathComponent("remote-account-fixture.json")
        guard FileManager.default.fileExists(atPath: marker.path) else { throw XCTSkip("Requires the explicit remote account fixture") }
        let requestFile = documents.appendingPathComponent("remote-account-request.json")
        let controller = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows).first { $0.isKeyWindow }?.rootViewController as? CAPBridgeViewController)
        let plugin = try XCTUnwrap(controller.bridge?.plugin(withName: "VelaRemote") as? VelaRemotePlugin)
        let original = try plugin.vault.read()
        defer {
            try? plugin.vault.write(original)
            try? FileManager.default.removeItem(at: requestFile)
            try? FileManager.default.removeItem(at: marker)
            controller.dismiss(animated: false)
        }
        try plugin.vault.update { $0.removeValue(forKey: "accountToken"); $0.removeValue(forKey: "accountAttempt") }
        _ = try await account(plugin, "login")
        XCTAssertTrue(controller.presentedViewController is SFSafariViewController)
        let attempt = try XCTUnwrap(plugin.vault.read()["accountAttempt"] as? [String: String])
        let request = ["code": try XCTUnwrap(attempt["code"]), "url": try XCTUnwrap(attempt["url"])]
        try JSONSerialization.data(withJSONObject: request).write(to: requestFile, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: requestFile.path)

        // A new native plugin instance must recover the pending attempt from Keychain.
        let restored = VelaRemotePlugin()
        let pending = try await account(restored, "status")
        XCTAssertEqual(pending["pending"] as? Bool, true)
        var linked = false
        for _ in 0..<180 {
            let result = try await account(restored, "poll")
            if result["linked"] as? Bool == true { linked = true; break }
            try await Task.sleep(for: .seconds(1))
        }
        XCTAssertTrue(linked, "Complete the fixture device authorization in vlx-browser")
        guard linked else { return }
        XCTAssertNil(try restored.vault.read()["accountAttempt"])
        XCTAssertNotNil(try restored.vault.read()["accountToken"])
        _ = try await account(plugin, "poll")
        try await Task.sleep(for: .seconds(1))
        XCTAssertNil(controller.presentedViewController)
        let status = try await account(VelaRemotePlugin(), "status")
        XCTAssertEqual(status["linked"] as? Bool, true)
        let devices = try await account(restored, "devices")
        let list = try XCTUnwrap(devices["devices"] as? [[String: Any]])
        XCTAssertFalse(list.isEmpty)
        let id = try XCTUnwrap(list.first?["id"] as? String)
        _ = try await account(plugin, "open", deviceId: id)
        XCTAssertTrue(controller.presentedViewController is SFSafariViewController)
        _ = try await account(restored, "logout")
        let loggedOut = try await account(VelaRemotePlugin(), "status")
        XCTAssertEqual(loggedOut["linked"] as? Bool, false)
        XCTAssertNil(try restored.vault.read()["accountToken"])
    }
}

final class SshIntegrationTests: XCTestCase {
    func testPasswordPrivateKeyHTTPAndWebSocketForwarding() async throws {
        let fixtureURL = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "ios", withExtension: "json"))
        let fixture = try JSONSerialization.jsonObject(with: Data(contentsOf: fixtureURL)) as! [String: Any]
        let plugin = VelaRemotePlugin()
        let original = try plugin.vault.read()
        let port = fixture["port"] as! Int
        var row: [String: Any] = ["id":"fixture", "name":"Fixture", "mode":"ssh", "host":"127.0.0.1", "port":port, "username":"fixture", "auth":"password", "password":"fixture-only-password", "service":"manual", "remotePort":fixture["httpPort"]!]
        try plugin.vault.write(["connections":[row], "keys":["127.0.0.1:\(port)":fixture["fingerprint"]!]])
        do {
            for mode in ["password", "key", "encrypted", "auto"] {
                row["auth"] = mode == "password" ? "password" : "key"
                row["privateKey"] = fixture[mode == "encrypted" ? "encryptedKey" : "privateKey"]
                row["passphrase"] = mode == "encrypted" ? "fixture-key-passphrase" : ""
                row["service"] = mode == "auto" ? "auto" : "manual"
                let (url, _) = try await plugin.establish(row, 0)
                let (body,response) = try await URLSession.shared.data(from: url.appendingPathComponent("api/mode"))
                XCTAssertEqual((response as? HTTPURLResponse)?.statusCode,200)
                XCTAssertTrue(String(decoding:body,as:UTF8.self).contains("fixture"))
                var components = URLComponents(url:url,resolvingAgainstBaseURL:false)!
                components.scheme="ws";components.path="/echo"
                let socket = URLSession.shared.webSocketTask(with:components.url!)
                socket.resume();try await socket.send(.string("tunnel-check"))
                let echo=try await socket.receive()
                if case .string(let value)=echo {XCTAssertEqual(value,"tunnel-check")} else {XCTFail("Expected text")}
                socket.cancel(with:.normalClosure,reason:nil)
                await plugin.closeTransport()
            }
            row["auth"]="password";row["password"]="wrong-password"
            do {_ = try await plugin.establish(row,0);XCTFail("Invalid password must fail")} catch {}
            await plugin.closeTransport()
            try plugin.vault.write(original)
        } catch {
            await plugin.closeTransport();try plugin.vault.write(original);throw error
        }
    }
}

final class ConnectionStorageTests: XCTestCase {
    func testCopyInheritsSecretsWithoutModifyingSourceOrReusingTunnel() {
        let source: [String: Any] = ["id":"original", "name":"Original", "mode":"ssh", "host":"one.test", "port":22, "username":"user", "auth":"key", "privateKey":"fixture-key", "passphrase":"fixture-passphrase", "webPassword":"fixture-web-password", "localPort":23191]
        var rows = [source]
        var draft = ConnectionRecords.summary(source)
        draft["id"] = nil; draft["name"] = "Second"; draft["host"] = "two.test"
        let copy = ConnectionRecords.upsert(ConnectionRecords.copied(draft, from: source), into: &rows, preserveEquivalent: true)
        XCTAssertEqual(rows.count, 2); XCTAssertNotEqual(copy["id"] as? String, source["id"] as? String)
        XCTAssertEqual(copy["localPort"] as? Int, 0)
        for key in ["privateKey", "passphrase", "webPassword"] {
            XCTAssertEqual(copy[key] as? String, source[key] as? String)
            XCTAssertNil(ConnectionRecords.summary(copy)[key])
        }
        XCTAssertEqual(rows[0]["host"] as? String, "one.test"); XCTAssertEqual(rows[0]["name"] as? String, "Original")
        XCTAssertEqual(rows[0]["localPort"] as? Int, 23191)
        draft["webPassword"] = "replacement"
        XCTAssertEqual(ConnectionRecords.copied(draft, from: source)["webPassword"] as? String, "replacement")
    }
    func testUnchangedCopyKeepsOriginalRecordIncludingName() {
        var source = url; source["id"] = "original"; source["name"] = "Original"
        var rows = [source]
        var draft = ConnectionRecords.summary(source); draft["name"] = "Copy"
        let saved = ConnectionRecords.upsert(ConnectionRecords.copied(draft, from: source), into: &rows, preserveEquivalent: true)
        XCTAssertEqual(rows.count, 1); XCTAssertEqual(saved["id"] as? String, "original"); XCTAssertEqual(saved["name"] as? String, "Original")
    }
    let url: [String: Any] = ["name": "Fixture", "mode": "url", "url": "https://example.test/#pair=fixture", "webPassword": "fixture-only-password"]
    func testDuplicateAndEditPreserveCredentials() {
        var rows = [[String: Any]]()
        let first = ConnectionRecords.upsert(ConnectionRecords.prepared(url, in: rows), into: &rows)
        var duplicate = url; duplicate["name"] = "Renamed"
        let second = ConnectionRecords.upsert(ConnectionRecords.prepared(duplicate, in: rows), into: &rows)
        XCTAssertEqual(first["id"] as? String, second["id"] as? String)
        XCTAssertEqual(rows.count, 1)
        var scanned = url; scanned["webPassword"] = ""
        let reused = ConnectionRecords.prepared(scanned, in: rows)
        XCTAssertEqual(reused["id"] as? String, second["id"] as? String)
        XCTAssertEqual(reused["webPassword"] as? String, "fixture-only-password")
        var edit = second; edit.removeValue(forKey: "webPassword")
        let prepared = ConnectionRecords.prepared(edit, in: rows)
        XCTAssertEqual(prepared["webPassword"] as? String, "fixture-only-password")
        XCTAssertNil(ConnectionRecords.summary(prepared)["webPassword"])
        XCTAssertEqual(ConnectionRecords.summary(prepared)["hasWebPassword"] as? Bool, true)
    }
    func testDifferentCredentialsAndPairingLinksRemainSeparate() {
        var rows = [[String: Any]]()
        for changes in [[String: String](), ["webPassword": "different"], ["url": "https://example.test/#pair=another"]] {
            let row = url.merging(changes) { _, new in new }
            _ = ConnectionRecords.upsert(ConnectionRecords.prepared(row, in: rows), into: &rows)
        }
        XCTAssertEqual(rows.count, 3)
        XCTAssertEqual(ConnectionRecords.unique(rows + rows).count, 3)
        var ssh: [String: Any] = ["mode":"ssh", "host":"EXAMPLE.test", "port":22, "username":"user", "auth":"password", "password":"fixture", "service":"manual", "remotePort":12345]
        let identity = ConnectionRecords.identity(ssh)
        ssh["host"] = "example.test"; ssh["privateKey"] = "unused"
        XCTAssertEqual(identity, ConnectionRecords.identity(ssh))
        ssh["username"] = "another"
        XCTAssertNotEqual(identity, ConnectionRecords.identity(ssh))
    }
    func testKeychainRoundTripAndPasswordRemoval() throws {
        let account = "storage-test-" + UUID().uuidString
        let vault = Vault(account: account)
        defer { SecItemDelete([kSecClass: kSecClassGenericPassword, kSecAttrService: "com.velaterm.mobile.remote", kSecAttrAccount: account] as CFDictionary) }
        try vault.write(["connections": [url], "keys": [String: String]()])
        let restored = try Vault(account: account).read()["connections"] as! [[String: Any]]
        XCTAssertEqual(restored[0]["webPassword"] as? String, "fixture-only-password")
        try vault.update { store in
            var rows = store["connections"] as! [[String: Any]]
            rows[0]["webPassword"] = ""; store["connections"] = rows
        }
        let cleared = try Vault(account: account).read()["connections"] as! [[String: Any]]
        XCTAssertEqual(cleared[0]["webPassword"] as? String, "")
    }
}

final class ConnectionRecoveryTests: XCTestCase {
    @MainActor func testReturnRemainsAvailableWhileRetrying() throws {
        let panel = ConnectionRecoveryView(frame: CGRect(x: 0, y: 0, width: 360, height: 640))
        var returned = false; var retried = false
        panel.onBack = { returned = true }; panel.onRetry = { retried = true }
        XCTAssertTrue(panel.isHidden)
        panel.showLoading(); panel.layoutIfNeeded()
        XCTAssertFalse(panel.isHidden); XCTAssertTrue(panel.retryButton.isHidden)
        XCTAssertTrue(panel.backButton.isEnabled)
        let loading = UIGraphicsImageRenderer(size: panel.bounds.size).image { context in panel.layer.render(in: context.cgContext) }
        let loadingFile = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("connection-loading.png")
        try loading.pngData()?.write(to: loadingFile)
        panel.showLoading(MobileText.get("mobile.loadSlow"), allowRetry: true)
        XCTAssertFalse(panel.retryButton.isHidden); XCTAssertTrue(panel.retryButton.isEnabled)
        XCTAssertTrue(panel.backButton.isEnabled)
        panel.show("无法加载远端页面，请检查网络后重试，或返回连接列表。")
        panel.layoutIfNeeded()
        XCTAssertFalse(panel.isHidden)
        XCTAssertGreaterThanOrEqual(panel.backButton.frame.height, 48)
        panel.retryButton.sendActions(for: .touchUpInside); XCTAssertTrue(retried)
        panel.show("正在恢复连接…", busy: true)
        XCTAssertFalse(panel.retryButton.isEnabled); XCTAssertTrue(panel.backButton.isEnabled)
        panel.backButton.sendActions(for: .touchUpInside); XCTAssertTrue(returned)
        panel.show("无法加载远端页面，请检查网络后重试，或返回连接列表。")
        panel.layoutIfNeeded()
        XCTAssertGreaterThanOrEqual(panel.backButton.frame.minY - panel.retryButton.frame.maxY, 16)
        let image = UIGraphicsImageRenderer(size: panel.bounds.size).image { context in panel.layer.render(in: context.cgContext) }
        let file = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("connection-recovery.png")
        try image.pngData()?.write(to: file)
    }
}
