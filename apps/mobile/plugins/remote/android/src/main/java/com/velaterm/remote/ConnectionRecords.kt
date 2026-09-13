package com.velaterm.remote

import org.json.JSONArray
import org.json.JSONObject
import java.util.Locale
import java.util.UUID

/** Identity includes credentials and pairing fragments, never presentation or tunnel state. */
internal object ConnectionRecords {
    val secrets = listOf("password", "privateKey", "passphrase", "webPassword")
    fun identity(row: JSONObject): List<String> {
        fun s(key: String) = row.optString(key, "")
        if (s("mode") == "url") return listOf("url", s("url"), s("webPassword"))
        return listOf(s("mode"), s("host").lowercase(Locale.ROOT), row.optInt("port", 0).toString(), s("username"), s("auth"),
            if (s("auth") == "key") s("privateKey") else s("password"), if (s("auth") == "key") s("passphrase") else "",
            s("service"), if (s("service") == "manual") row.optInt("remotePort", 0).toString() else "",
            if (s("service") == "auto") row.optBoolean("prepare", false).toString() else "", s("webPassword"))
    }
    fun prepared(input: JSONObject, rows: JSONArray): JSONObject {
        val row = JSONObject(input.toString())
        val items = (0 until rows.length()).map { rows.getJSONObject(it) }
        var old = items.firstOrNull { it.optString("id") == input.optString("id") }
        // Re-scanning a saved address preserves its secret when there is one unambiguous match.
        if (old == null && input.optString("id").isEmpty() && secrets.all { input.optString(it, "").isEmpty() }) {
            fun withoutSecrets(value: JSONObject) = identity(JSONObject(value.toString()).apply { secrets.forEach { remove(it) } })
            val matches = items.filter { withoutSecrets(it) == withoutSecrets(input) }
            if (matches.size == 1) { old = matches[0]; secrets.forEach { row.remove(it) } }
        }
        for (key in secrets) if (!row.has(key) && old != null) row.put(key, old.optString(key, ""))
        row.put("id", old?.getString("id") ?: UUID.randomUUID().toString())
        row.put("localPort", old?.optInt("localPort", 0) ?: 0)
        return row
    }
    fun copied(input: JSONObject, source: JSONObject): JSONObject = JSONObject(input.toString()).apply {
        for (key in secrets) if (!has(key)) put(key, source.optString(key, ""))
        put("id", UUID.randomUUID().toString())
        put("localPort", 0)
    }
    fun upsert(input: JSONObject, rows: JSONArray, preserveEquivalent: Boolean = false): Pair<JSONObject, JSONArray> {
        val row = JSONObject(input.toString())
        val items = (0 until rows.length()).map { rows.getJSONObject(it) }
        val same = items.firstOrNull { identity(it) == identity(row) }
        // Saving an unchanged copy must not rename or otherwise overwrite its source.
        if (preserveEquivalent && same != null) return same to rows
        if (items.none { it.optString("id") == row.optString("id") } && same != null) {
            row.put("id", same.getString("id")); row.put("localPort", same.optInt("localPort", 0))
        }
        val index = items.indexOfFirst { it.optString("id") == row.optString("id") }.let { if (it < 0) items.size else it }
        val result = items.filterNot { it.optString("id") == row.optString("id") || identity(it) == identity(row) }.toMutableList()
        result.add(minOf(index, result.size), row)
        return row to JSONArray(result)
    }
    fun unique(rows: JSONArray) = JSONArray((0 until rows.length()).map { rows.getJSONObject(it) }.distinctBy { identity(it) })
    fun summary(row: JSONObject): JSONObject = JSONObject(row.toString()).apply {
        put("hasSecret", optString("password").isNotEmpty() || optString("privateKey").isNotEmpty())
        put("hasWebPassword", optString("webPassword").isNotEmpty())
        secrets.forEach { remove(it) }
    }
}
