import XCTest
import Security
import UIKit
import Capacitor
import SafariServices
import WebKit
@testable import VelaRemotePlugin

/// Opt-in native account checks against the public service. Authorization is completed in the dedicated vlx-browser profile.
final class RemoteAccountIntegrationTests: XCTestCase {
    @MainActor private func account(_ plugin: VelaRemotePlugin, _ action: String, deviceId: String? = nil, grantId: String? = nil) async throws -> [String: Any] {
        try await withCheckedThrowingContinuation { continuation in
            var options: [String: Any] = ["action": action]
            if let deviceId { options["deviceId"] = deviceId }
            if let grantId { options["grantId"] = grantId }
            let call = CAPPluginCall(callbackId: UUID().uuidString, methodName: "account", options: options, success: { result, _ in
                continuation.resume(returning: result?.data ?? [:])
            }, error: { error in
                continuation.resume(throwing: NSError(domain: error?.code ?? "account", code: 1, userInfo: [NSLocalizedDescriptionKey: error?.message ?? "Account request failed"]))
            })!
            plugin.account(call)
        }
    }

    @MainActor private func connection(_ plugin: VelaRemotePlugin, _ action: String, id: String? = nil) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            var options: [String: Any] = [:]
            if let id { options["id"] = id }
            let call = CAPPluginCall(callbackId: UUID().uuidString, methodName: action, options: options, success: { _, _ in
                continuation.resume()
            }, error: { error in
                continuation.resume(throwing: NSError(domain: error?.code ?? "connection", code: 1, userInfo: [NSLocalizedDescriptionKey: "Native connection failed"]))
            })!
            if action == "connect" { plugin.connect(call) } else { plugin.disconnect(call) }
        }
    }

    @MainActor private func renderedProject(_ controller: UIViewController, name: String) async throws {
        func webView(_ view: UIView) -> WKWebView? {
            if let web = view as? WKWebView { return web }
            for child in view.subviews { if let web = webView(child) { return web } }
            return nil
        }
        let presented = try XCTUnwrap(controller.presentedViewController)
        let web = try XCTUnwrap(webView(presented.view))
        let encoded = String(decoding: try JSONSerialization.data(withJSONObject: name, options: .fragmentsAllowed), as: UTF8.self)
        let expression = "!!document.querySelector('.m-app .m-header') && !document.querySelector('.m-load-status, .m-list[aria-busy=\"true\"]') && document.body.innerText.includes(\(encoded))"
        for _ in 0..<60 {
            if (try? await web.evaluateJavaScript(expression)) as? Bool == true { return }
            try await Task.sleep(for: .seconds(1))
        }
        XCTFail("The expected remote project did not render")
    }

    @MainActor func testNativeAuthorizationRecoveryAndLogout() async throws {
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let marker = documents.appendingPathComponent("remote-account-fixture.json")
        guard FileManager.default.fileExists(atPath: marker.path) else { throw XCTSkip("Requires the explicit remote account fixture") }
        let fixture = try JSONSerialization.jsonObject(with: Data(contentsOf: marker)) as? [String: String] ?? [:]
        let requestFile = documents.appendingPathComponent("remote-account-request.json")
        let controller = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows).first { $0.isKeyWindow }?.rootViewController as? CAPBridgeViewController)
        let plugin = try XCTUnwrap(controller.bridge?.plugin(withName: "VelaRemote") as? VelaRemotePlugin)
        let original = try plugin.vault.read()
        defer {
            plugin.cancelAccountLogin()
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
            // Do not poll from the test: Safari must close even when WebView timers stop.
            let stored = try plugin.vault.read()
            if stored["accountToken"] != nil && stored["accountAttempt"] == nil && controller.presentedViewController == nil { linked = true; break }
            try await Task.sleep(for: .seconds(1))
        }
        XCTAssertTrue(linked, "Complete the fixture device authorization in vlx-browser")
        guard linked else { return }
        XCTAssertNil(try restored.vault.read()["accountAttempt"])
        XCTAssertNotNil(try restored.vault.read()["accountToken"])
        let completed = try await account(plugin, "poll")
        XCTAssertEqual(completed["linked"] as? Bool, true)
        try await Task.sleep(for: .seconds(1))
        XCTAssertNil(controller.presentedViewController)
        let status = try await account(VelaRemotePlugin(), "status")
        XCTAssertEqual(status["linked"] as? Bool, true)
        let devices = try await account(restored, "devices")
        let list = try XCTUnwrap(devices["devices"] as? [[String: Any]])
        XCTAssertFalse(list.isEmpty)
        if let name = fixture["projectName"] {
            let access = list.flatMap { $0["access"] as? [[String: Any]] ?? [] }
            let grant = try XCTUnwrap(access.first { $0["name"] as? String == name && $0["ready"] as? Bool == true })
            _ = try await account(plugin, "open", grantId: try XCTUnwrap(grant["id"] as? String))
            try await renderedProject(controller, name: name)
        } else {
            let id = try XCTUnwrap(list.first?["id"] as? String)
            _ = try await account(plugin, "open", deviceId: id)
            XCTAssertNotNil(controller.presentedViewController)
        }
        try await connection(plugin, "disconnect")
        _ = try await account(restored, "logout")
        let loggedOut = try await account(VelaRemotePlugin(), "status")
        XCTAssertEqual(loggedOut["linked"] as? Bool, false)
        XCTAssertNil(try restored.vault.read()["accountToken"])
    }

    @MainActor func testReviewHostSSHAndURL() async throws {
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let marker = documents.appendingPathComponent("review-host-fixture.json")
        guard FileManager.default.fileExists(atPath: marker.path) else { throw XCTSkip("Requires the explicit review host fixture") }
        let fixture = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(contentsOf: marker)) as? [String: Any])
        let rows = try XCTUnwrap(fixture["connections"] as? [[String: Any]])
        let name = try XCTUnwrap(fixture["projectName"] as? String)
        let controller = try XCTUnwrap(UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows).first { $0.isKeyWindow }?.rootViewController as? CAPBridgeViewController)
        let plugin = try XCTUnwrap(controller.bridge?.plugin(withName: "VelaRemote") as? VelaRemotePlugin)
        let original = try plugin.vault.read()
        defer { try? plugin.vault.write(original); try? FileManager.default.removeItem(at: marker) }
        try plugin.vault.update { store in
            store["connections"] = rows
            var keys = store["keys"] as? [String: String] ?? [:]
            keys.merge(fixture["keys"] as? [String: String] ?? [:]) { _, fixture in fixture }
            store["keys"] = keys
        }
        do {
            for row in rows {
                try await connection(plugin, "connect", id: try XCTUnwrap(row["id"] as? String))
                try await renderedProject(controller, name: name)
                try await connection(plugin, "disconnect")
            }
        } catch {
            try? await connection(plugin, "disconnect")
            throw error
        }
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
    @MainActor func testLocalizedRecoveryLayoutForAllElevenLanguages() throws {
        let defaults = UserDefaults.standard
        let domain = Bundle.main.bundleIdentifier!
        let original = defaults.persistentDomain(forName: domain)?["AppleLanguages"]
        defer { if let original { defaults.set(original, forKey: "AppleLanguages") } else { defaults.removeObject(forKey: "AppleLanguages") } }
        let directory = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        for locale in ["en", "zh-CN", "zh-TW", "ja", "ko", "fr", "de", "es", "pt-BR", "ru", "vi"] {
            defaults.set([locale], forKey: "AppleLanguages")
            XCTAssertEqual(Locale.preferredLanguages.first, locale)
            let panel = ConnectionRecoveryView(frame: CGRect(x: 0, y: 0, width: 320, height: 640))
            panel.show(MobileText.get("mobile.native.certificateRejected")); panel.layoutIfNeeded()
            XCTAssertGreaterThanOrEqual(panel.backButton.frame.minY - panel.retryButton.frame.maxY, 16, locale)
            for button in [panel.retryButton, panel.backButton] {
                let label = try XCTUnwrap(button.titleLabel)
                XCTAssertFalse((label.text ?? "").contains("mobile."), locale)
                let measured = label.sizeThatFits(CGSize(width: label.bounds.width, height: .greatestFiniteMagnitude))
                XCTAssertLessThanOrEqual(measured.height, label.bounds.height + 1, locale)
                XCTAssertLessThanOrEqual(measured.width, label.bounds.width + 1, locale)
            }
            let image = UIGraphicsImageRenderer(size: panel.bounds.size).image { context in panel.layer.render(in: context.cgContext) }
            try image.pngData()?.write(to: directory.appendingPathComponent("recovery-" + locale + ".png"))
            let localization = locale == "zh-CN" ? "zh-Hans" : locale == "zh-TW" ? "zh-Hant" : locale
            let bundle = try XCTUnwrap(Bundle(path: Bundle.main.bundlePath + "/" + localization + ".lproj"))
            let privacy = bundle.localizedString(forKey: "NSCameraUsageDescription", value: nil, table: "InfoPlist")
            XCTAssertNotEqual(privacy, "NSCameraUsageDescription", locale)
            XCTAssertEqual(privacy, MobileText.get("mobile.native.cameraUsageDescription"), locale)
        }
    }

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
