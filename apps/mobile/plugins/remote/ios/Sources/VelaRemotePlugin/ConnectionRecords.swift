import Foundation

/// Connection identity includes credentials and pairing fragments, never presentation or tunnel state.
enum ConnectionRecords {
    static let secrets = ["password", "privateKey", "passphrase", "webPassword"]
    static func identity(_ row: [String: Any]) -> [String] {
        func s(_ key: String) -> String { row[key] as? String ?? "" }
        if s("mode") == "url" { return ["url", s("url"), s("webPassword")] }
        return [s("mode"), s("host").lowercased(), String((row["port"] as? NSNumber)?.intValue ?? 0), s("username"), s("auth"),
                s("auth") == "key" ? s("privateKey") : s("password"), s("auth") == "key" ? s("passphrase") : "",
                s("service"), s("service") == "manual" ? String((row["remotePort"] as? NSNumber)?.intValue ?? 0) : "",
                s("service") == "auto" ? String(row["prepare"] as? Bool ?? false) : "", s("webPassword")]
    }
    static func prepared(_ input: [String: Any], in rows: [[String: Any]]) -> [String: Any] {
        var row = input
        var old = rows.first { ($0["id"] as? String) == (input["id"] as? String) }
        // Re-scanning a saved address does not ask the user to re-enter an existing secret.
        if old == nil, (input["id"] as? String ?? "").isEmpty, secrets.allSatisfy({ (input[$0] as? String ?? "").isEmpty }) {
            func withoutSecrets(_ value: [String: Any]) -> [String] {
                var value = value; for key in secrets { value.removeValue(forKey: key) }; return identity(value)
            }
            let matches = rows.filter { withoutSecrets($0) == withoutSecrets(input) }
            if matches.count == 1 { old = matches[0]; for key in secrets { row.removeValue(forKey: key) } }
        }
        for key in secrets where row[key] == nil { row[key] = old?[key] }
        row["id"] = old?["id"] ?? UUID().uuidString
        row["localPort"] = old?["localPort"] ?? 0
        return row
    }
    static func copied(_ input: [String: Any], from source: [String: Any]) -> [String: Any] {
        var row = input
        for key in secrets where row[key] == nil { row[key] = source[key] }
        row["id"] = UUID().uuidString
        row["localPort"] = 0
        return row
    }
    static func upsert(_ input: [String: Any], into rows: inout [[String: Any]], preserveEquivalent: Bool = false) -> [String: Any] {
        var row = input
        let same = rows.first { identity($0) == identity(row) }
        // Saving an unchanged copy must not rename or otherwise overwrite its source.
        if preserveEquivalent, let same { return same }
        let existing = rows.first { ($0["id"] as? String) == (row["id"] as? String) }
        if existing == nil, let same { row["id"] = same["id"]; row["localPort"] = same["localPort"] }
        let index = rows.firstIndex { ($0["id"] as? String) == (row["id"] as? String) } ?? rows.count
        rows.removeAll { ($0["id"] as? String) == (row["id"] as? String) || identity($0) == identity(row) }
        rows.insert(row, at: min(index, rows.count))
        return row
    }
    static func unique(_ rows: [[String: Any]]) -> [[String: Any]] {
        var result = [[String: Any]]()
        for row in rows where !result.contains(where: { identity($0) == identity(row) }) { result.append(row) }
        return result
    }
    static func summary(_ row: [String: Any]) -> [String: Any] {
        var result = row
        result["hasSecret"] = !(row["password"] as? String ?? "").isEmpty || !(row["privateKey"] as? String ?? "").isEmpty
        result["hasWebPassword"] = !(row["webPassword"] as? String ?? "").isEmpty
        for key in secrets { result.removeValue(forKey: key) }
        return result
    }
}
