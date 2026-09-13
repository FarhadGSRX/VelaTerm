package com.velaterm.remote

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

class ConnectionRecordsTest {
    @Test fun copyInheritsSecretsWithoutModifyingSourceOrReusingTunnel() {
        val source = JSONObject().put("id", "original").put("name", "Original").put("mode", "ssh").put("host", "one.test")
            .put("port", 22).put("username", "user").put("auth", "key").put("privateKey", "fixture-key")
            .put("passphrase", "fixture-passphrase").put("webPassword", "fixture-web-password").put("localPort", 23191)
        val rows = JSONArray().put(source)
        val draft = ConnectionRecords.summary(source).put("name", "Second").put("host", "two.test")
        draft.remove("id")
        val (copy, saved) = ConnectionRecords.upsert(ConnectionRecords.copied(draft, source), rows, preserveEquivalent = true)
        assertEquals(2, saved.length()); assertNotEquals(source.getString("id"), copy.getString("id"))
        assertEquals(0, copy.getInt("localPort"))
        for (key in listOf("privateKey", "passphrase", "webPassword")) {
            assertEquals(source.getString(key), copy.getString(key)); assertFalse(ConnectionRecords.summary(copy).has(key))
        }
        assertEquals("one.test", source.getString("host")); assertEquals("Original", source.getString("name"))
        assertEquals(23191, source.getInt("localPort"))
        assertEquals("replacement", ConnectionRecords.copied(draft.put("webPassword", "replacement"), source).getString("webPassword"))
    }
    @Test fun unchangedCopyKeepsOriginalRecordIncludingName() {
        val source = fixture().put("id", "original").put("name", "Original")
        val draft = ConnectionRecords.summary(source).put("name", "Copy")
        val (saved, rows) = ConnectionRecords.upsert(ConnectionRecords.copied(draft, source), JSONArray().put(source), preserveEquivalent = true)
        assertEquals(1, rows.length()); assertEquals("original", saved.getString("id")); assertEquals("Original", saved.getString("name"))
    }
    private fun fixture() = JSONObject().put("name", "Fixture").put("mode", "url").put("url", "https://example.test/#pair=fixture").put("webPassword", "fixture-only-password")
    @Test fun duplicateAndEditPreserveCredentials() {
        val (first, rows) = ConnectionRecords.upsert(ConnectionRecords.prepared(fixture(), JSONArray()), JSONArray())
        val (second, saved) = ConnectionRecords.upsert(ConnectionRecords.prepared(fixture().put("name", "Renamed"), rows), rows)
        assertEquals(first.getString("id"), second.getString("id")); assertEquals(1, saved.length())
        val reused = ConnectionRecords.prepared(fixture().put("webPassword", ""), rows)
        assertEquals(second.getString("id"), reused.getString("id"))
        assertEquals("fixture-only-password", reused.getString("webPassword"))
        second.remove("webPassword")
        val edit = ConnectionRecords.prepared(second, rows)
        assertEquals("fixture-only-password", edit.getString("webPassword"))
        assertFalse(ConnectionRecords.summary(edit).has("webPassword"))
        assertTrue(ConnectionRecords.summary(edit).getBoolean("hasWebPassword"))
    }
    @Test fun differentCredentialsAndPairingLinksRemainSeparate() {
        var rows = JSONArray()
        for (input in listOf(fixture(), fixture().put("webPassword", "different"), fixture().put("url", "https://example.test/#pair=another"))) {
            rows = ConnectionRecords.upsert(ConnectionRecords.prepared(input, rows), rows).second
        }
        assertEquals(3, rows.length())
        assertEquals(3, ConnectionRecords.unique(JSONArray((0..5).map { rows.getJSONObject(it % 3) })).length())
        val ssh = JSONObject().put("mode", "ssh").put("host", "EXAMPLE.test").put("port", 22).put("username", "user").put("auth", "password").put("password", "fixture").put("service", "manual").put("remotePort", 12345)
        val identity = ConnectionRecords.identity(ssh)
        ssh.put("host", "example.test").put("privateKey", "unused")
        assertEquals(identity, ConnectionRecords.identity(ssh))
        ssh.put("username", "another")
        assertNotEquals(identity, ConnectionRecords.identity(ssh))
    }
}
