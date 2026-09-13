package com.velaterm.mobile;

import androidx.test.core.app.ActivityScenario;
import androidx.test.platform.app.InstrumentationRegistry;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import com.getcapacitor.JSObject;
import com.getcapacitor.PluginCall;
import com.velaterm.remote.VelaRemotePlugin;
import org.json.JSONObject;
import org.junit.Test;
import org.junit.runner.RunWith;
import java.io.File;
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import static org.junit.Assert.*;
import static org.junit.Assume.assumeTrue;

/** Opt-in native account verification; website authorization uses the dedicated vlx-browser profile. */
@RunWith(AndroidJUnit4.class)
public class RemoteAccountIntegrationTest {
    private Object invoke(Object plugin, String method, Class<?>[] types, Object... args) throws Exception {
        Method m = plugin.getClass().getDeclaredMethod(method, types);
        m.setAccessible(true);
        return m.invoke(plugin, args);
    }
    private JSObject account(VelaRemotePlugin plugin, String action, String deviceId) throws Exception {
        CompletableFuture<JSObject> result = new CompletableFuture<>();
        JSObject options = new JSObject().put("action", action);
        if (deviceId != null) options.put("deviceId", deviceId);
        PluginCall call = new PluginCall(null, "VelaRemote", "fixture", "account", options) {
            @Override public void resolve(JSObject value) { result.complete(value); }
            @Override public void resolve() { result.complete(new JSObject()); }
            @Override public void reject(String message, String code, Exception error, JSObject data) {
                result.completeExceptionally(new IllegalStateException(code + ": " + message));
            }
        };
        plugin.account(call);
        return result.get(60, TimeUnit.SECONDS);
    }
    @Test public void nativeAuthorizationRecoveryAndLogout() throws Exception {
        assumeTrue("Requires explicit live account verification", "true".equals(InstrumentationRegistry.getArguments().getString("remoteAccount")));
        try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(MainActivity.class)) {
            AtomicReference<VelaRemotePlugin> instance = new AtomicReference<>();
            scenario.onActivity(activity -> instance.set((VelaRemotePlugin) activity.getBridge().getPlugin("VelaRemote").getInstance()));
            VelaRemotePlugin plugin = instance.get();
            JSONObject original = (JSONObject) invoke(plugin, "readStore", new Class<?>[]{});
            File requestFile = new File(InstrumentationRegistry.getInstrumentation().getTargetContext().getFilesDir(), "remote-account-request.json");
            try {
                JSONObject empty = new JSONObject(original.toString());
                empty.remove("accountToken"); empty.remove("accountAttempt");
                invoke(plugin, "writeStore", new Class<?>[]{JSONObject.class}, empty);
                account(plugin, "login", null);
                JSONObject saved = (JSONObject) invoke(plugin, "readStore", new Class<?>[]{});
                JSONObject attempt = saved.getJSONObject("accountAttempt");
                Files.write(requestFile.toPath(), new JSONObject().put("code", attempt.getString("code")).put("url", attempt.getString("url")).toString().getBytes(StandardCharsets.UTF_8));
                String encrypted = InstrumentationRegistry.getInstrumentation().getTargetContext().getSharedPreferences("vela-remote", 0).getString("vault", "");
                assertFalse(encrypted.contains(attempt.getString("pollToken")));
                VelaRemotePlugin restored = new VelaRemotePlugin();
                restored.setBridge(plugin.getBridge());
                assertTrue(account(restored, "status", null).getBoolean("pending"));
                boolean linked = false;
                for (int i = 0; i < 180; i++) {
                    if (account(restored, "poll", null).getBoolean("linked")) { linked = true; break; }
                    Thread.sleep(1000);
                }
                assertTrue("Complete the fixture device authorization in vlx-browser", linked);
                InstrumentationRegistry.getInstrumentation().getUiAutomation().performGlobalAction(android.accessibilityservice.AccessibilityService.GLOBAL_ACTION_BACK);
                InstrumentationRegistry.getInstrumentation().waitForIdleSync();
                Thread.sleep(1000);
                JSONObject linkedStore = (JSONObject) invoke(restored, "readStore", new Class<?>[]{});
                assertFalse(linkedStore.has("accountAttempt"));
                assertTrue(linkedStore.has("accountToken"));
                assertTrue(account(restored, "status", null).getBoolean("linked"));
                var devices = account(restored, "devices", null).getJSONArray("devices");
                assertTrue(devices.length() > 0);
                account(restored, "open", devices.getJSONObject(0).getString("id"));
                InstrumentationRegistry.getInstrumentation().waitForIdleSync();
                Thread.sleep(1000);
                InstrumentationRegistry.getInstrumentation().getUiAutomation().performGlobalAction(android.accessibilityservice.AccessibilityService.GLOBAL_ACTION_BACK);
                InstrumentationRegistry.getInstrumentation().waitForIdleSync();
                Thread.sleep(1000);
                account(restored, "logout", null);
                assertFalse(account(restored, "status", null).getBoolean("linked"));
                assertFalse(((JSONObject) invoke(restored, "readStore", new Class<?>[]{})).has("accountToken"));
            } finally {
                requestFile.delete();
                invoke(plugin, "writeStore", new Class<?>[]{JSONObject.class}, original);
            }
        }
    }
}
