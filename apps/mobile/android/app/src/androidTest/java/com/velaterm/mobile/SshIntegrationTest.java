package com.velaterm.mobile;

import androidx.test.core.app.ActivityScenario;
import androidx.test.platform.app.InstrumentationRegistry;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.json.JSONObject;
import org.json.JSONArray;
import java.lang.reflect.Method;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.atomic.AtomicReference;
import kotlin.Pair;
import static org.junit.Assert.*;

/** Exercises the production native transport against a local SSH fixture; no WebView automation. */
@RunWith(AndroidJUnit4.class)
public class SshIntegrationTest {
    @Test public void nativeTaskNotificationCarriesConnectionAndSession() throws Exception {
        try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(MainActivity.class)) {
            AtomicReference<Throwable> failure = new AtomicReference<>();
            scenario.onActivity(activity -> {
                try {
                    Object plugin = activity.getBridge().getPlugin("VelaRemote").getInstance();
                    var field = plugin.getClass().getDeclaredField("notifications"); field.setAccessible(true);
                    Object notifications = field.get(plugin);
                    invoke(notifications, "send", new Class<?>[]{String.class, JSONObject.class}, "notification-fixture",
                        new JSONObject().put("title", "Fixture task").put("body", "已修复通知跳转，点击可查看对应会话。🙂").put("sessionId", "session-fixture").put("sound", true));
                } catch (Throwable error) { failure.set(error); }
            });
            if (failure.get() != null) throw new AssertionError(failure.get());
            var context = InstrumentationRegistry.getInstrumentation().getTargetContext();
            var manager = context.getSystemService(android.app.NotificationManager.class);
            try {
                android.service.notification.StatusBarNotification delivered = null;
                for (int i = 0; i < 40 && delivered == null; i++) {
                    for (var notification : manager.getActiveNotifications()) if ("notification-fixture:session-fixture".equals(notification.getTag())) delivered = notification;
                    if (delivered == null) Thread.sleep(50);
                }
                assertNotNull("The native notification must reach Android", delivered);
                assertEquals("Fixture task", delivered.getNotification().extras.getString(android.app.Notification.EXTRA_TITLE));
                assertEquals("已修复通知跳转，点击可查看对应会话。🙂", delivered.getNotification().extras.getCharSequence(android.app.Notification.EXTRA_TEXT).toString());
                assertEquals("已修复通知跳转，点击可查看对应会话。🙂", delivered.getNotification().extras.getCharSequence(android.app.Notification.EXTRA_BIG_TEXT).toString());
                assertNotNull(delivered.getNotification().contentIntent);
                assertEquals(context.getPackageName(), delivered.getNotification().contentIntent.getCreatorPackage());
                // Retain the native event for inspection instead of asking the fixture WebView to connect.
                scenario.onActivity(activity -> {
                    try {
                        Object plugin = activity.getBridge().getPlugin("VelaRemote").getInstance();
                        var listeners = com.getcapacitor.Plugin.class.getDeclaredField("eventListeners"); listeners.setAccessible(true);
                        ((java.util.Map<?, ?>) listeners.get(plugin)).remove("notificationOpen");
                    } catch (Throwable error) { failure.set(error); }
                });
                if (failure.get() != null) throw new AssertionError(failure.get());
                delivered.getNotification().contentIntent.send();
                AtomicReference<JSONObject> opened = new AtomicReference<>();
                for (int i = 0; i < 40 && opened.get() == null; i++) {
                    scenario.onActivity(activity -> {
                        try {
                            Object plugin = activity.getBridge().getPlugin("VelaRemote").getInstance();
                            var retained = com.getcapacitor.Plugin.class.getDeclaredField("retainedEventArguments"); retained.setAccessible(true);
                            Object events = ((java.util.Map<?, ?>) retained.get(plugin)).get("notificationOpen");
                            if (events instanceof java.util.List<?> && !((java.util.List<?>) events).isEmpty())
                                opened.set((JSONObject) ((java.util.List<?>) events).get(0));
                        } catch (Throwable error) { failure.set(error); }
                    });
                    if (opened.get() == null) Thread.sleep(50);
                }
                if (failure.get() != null) throw new AssertionError(failure.get());
                assertNotNull("Tapping the system notification must reach the native connection router", opened.get());
                assertEquals("notification-fixture", opened.get().getString("id"));
                assertEquals("session-fixture", opened.get().getString("sessionId"));
            } finally { manager.cancel("notification-fixture:session-fixture", 1); }
        }
    }
    @Test public void returnRemainsAvailableWhileRetrying() {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(() -> {
            var view = new com.velaterm.remote.ConnectionRecoveryView(InstrumentationRegistry.getInstrumentation().getTargetContext());
            var returned = new java.util.concurrent.atomic.AtomicBoolean(false);
            var retried = new java.util.concurrent.atomic.AtomicBoolean(false);
            view.setOnBack(() -> { returned.set(true); return kotlin.Unit.INSTANCE; });
            view.setOnRetry(() -> { retried.set(true); return kotlin.Unit.INSTANCE; });
            assertEquals(android.view.View.GONE, view.getVisibility());
            view.showLoading("", false);
            assertEquals(android.view.View.VISIBLE, view.getVisibility());
            assertEquals(android.view.View.GONE, view.getRetryButton().getVisibility());
            assertTrue(view.getBackButton().isEnabled());
            saveRecoverySnapshot(view, "connection-loading.png");
            view.showLoading("Loading is taking longer than expected.", true);
            assertEquals(android.view.View.VISIBLE, view.getRetryButton().getVisibility());
            assertTrue(view.getRetryButton().isEnabled()); assertTrue(view.getBackButton().isEnabled());
            view.show("无法加载远端页面", false);
            assertEquals(android.view.View.VISIBLE, view.getVisibility());
            view.getRetryButton().performClick(); assertTrue(retried.get());
            view.show("正在恢复连接…", true);
            assertFalse(view.getRetryButton().isEnabled()); assertTrue(view.getBackButton().isEnabled());
            view.getBackButton().performClick(); assertTrue(returned.get());
            view.show("Unable to load the remote page. Retry or return to your connections.", false);
            saveRecoverySnapshot(view, "connection-recovery.png");
            assertTrue(view.getBackButton().getTop() - view.getRetryButton().getBottom() >= 16 * view.getResources().getDisplayMetrics().density);
        });
    }
    private void saveRecoverySnapshot(android.view.View view, String name) {
        int width = (int) (360 * view.getResources().getDisplayMetrics().density);
        int height = (int) (640 * view.getResources().getDisplayMetrics().density);
        view.measure(android.view.View.MeasureSpec.makeMeasureSpec(width, android.view.View.MeasureSpec.EXACTLY), android.view.View.MeasureSpec.makeMeasureSpec(height, android.view.View.MeasureSpec.EXACTLY));
        view.layout(0, 0, width, height);
        android.graphics.Bitmap bitmap = android.graphics.Bitmap.createBitmap(width, height, android.graphics.Bitmap.Config.ARGB_8888);
        view.draw(new android.graphics.Canvas(bitmap));
        try (var output = view.getContext().openFileOutput(name, android.content.Context.MODE_PRIVATE)) { bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output); }
        catch (java.io.IOException error) { throw new AssertionError(error); }
        finally { bitmap.recycle(); }
    }
    private Object invoke(Object plugin, String method, Class<?>[] types, Object... args) throws Exception {
        Method m = plugin.getClass().getDeclaredMethod(method, types); m.setAccessible(true); return m.invoke(plugin, args);
    }
    @Test public void savedPasswordSurvivesKeystoreRoundTrip() throws Exception {
        try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(MainActivity.class)) {
            AtomicReference<Object> instance = new AtomicReference<>();
            scenario.onActivity(activity -> instance.set(activity.getBridge().getPlugin("VelaRemote").getInstance()));
            Object plugin = instance.get();
            JSONObject original = (JSONObject) invoke(plugin, "readStore", new Class<?>[]{});
            try {
                JSONObject row = new JSONObject().put("id", "storage-fixture").put("name", "Fixture").put("mode", "url").put("url", "https://example.test/#pair=fixture");
                invoke(plugin, "writeStore", new Class<?>[]{JSONObject.class}, new JSONObject().put("connections", new JSONArray().put(row)).put("keys", new JSONObject()));
                invoke(plugin, "saveWebPassword", new Class<?>[]{String.class, String.class}, "storage-fixture", "fixture-only-password");
                JSONObject restored = (JSONObject) invoke(plugin, "readStore", new Class<?>[]{});
                assertEquals("fixture-only-password", restored.getJSONArray("connections").getJSONObject(0).getString("webPassword"));
                String encrypted = InstrumentationRegistry.getInstrumentation().getTargetContext().getSharedPreferences("vela-remote", 0).getString("vault", "");
                assertFalse(encrypted.contains("fixture-only-password"));
                invoke(plugin, "saveWebPassword", new Class<?>[]{String.class, String.class}, "storage-fixture", "");
                JSONObject cleared = (JSONObject) invoke(plugin, "readStore", new Class<?>[]{});
                assertEquals("", cleared.getJSONArray("connections").getJSONObject(0).getString("webPassword"));
            } finally { invoke(plugin, "writeStore", new Class<?>[]{JSONObject.class}, original); }
        }
    }
    @Test public void passwordAndKeyForwardHttpThroughProductionPlugin() throws Exception {
        var args = InstrumentationRegistry.getArguments();
        assertNotNull("Run with the local fixture parameters", args.getString("sshPort"));
        try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(MainActivity.class)) {
            AtomicReference<Object> instance = new AtomicReference<>();
            scenario.onActivity(activity -> instance.set(activity.getBridge().getPlugin("VelaRemote").getInstance()));
            Object plugin = instance.get(); assertNotNull(plugin);
            JSONObject original = (JSONObject) invoke(plugin, "readStore", new Class<?>[]{});
            try {
                String host="10.0.2.2";
                int sshPort=Integer.parseInt(args.getString("sshPort"));
                int httpPort=Integer.parseInt(args.getString("httpPort"));
                JSONObject row=new JSONObject().put("id","fixture").put("name","Fixture").put("mode","ssh").put("host",host).put("port",sshPort).put("username","fixture").put("auth","password").put("password","fixture-only-password").put("service","manual").put("remotePort",httpPort);
                JSONObject store=new JSONObject().put("connections",new JSONArray().put(row)).put("keys",new JSONObject().put(host+":"+sshPort,args.getString("fingerprint")));
                invoke(plugin,"writeStore",new Class<?>[]{JSONObject.class},store);
                for (String mode : new String[]{"password","key","encrypted","auto"}) {
                    row.put("auth",mode.equals("password") ? "password" : "key");
                    String filename=mode.equals("encrypted") ? "fixture_encrypted_key" : "fixture_key";
                    try(var input=InstrumentationRegistry.getInstrumentation().getTargetContext().openFileInput(filename)) {
                        row.put("privateKey",new String(input.readAllBytes(),StandardCharsets.UTF_8));
                    }
                    row.put("passphrase",mode.equals("encrypted") ? "fixture-key-passphrase" : "");
                    row.put("service",mode.equals("auto") ? "auto" : "manual");
                    Pair<?,?> result=(Pair<?,?>)invoke(plugin,"establish",new Class<?>[]{JSONObject.class,int.class},row,0);
                    try(var stream=new URL(result.getFirst()+"/api/mode").openStream()) {
                        assertTrue(new String(stream.readAllBytes(),StandardCharsets.UTF_8).contains("fixture"));
                    }
                    invoke(plugin,"closeTransport",new Class<?>[]{});
                }
                row.put("auth","password").put("password","wrong-password");
                try {invoke(plugin,"establish",new Class<?>[]{JSONObject.class,int.class},row,0);fail("Invalid password must fail");} catch(java.lang.reflect.InvocationTargetException expected){assertNotNull(expected.getCause());}
            } finally {invoke(plugin,"closeTransport",new Class<?>[]{});invoke(plugin,"writeStore",new Class<?>[]{JSONObject.class},original);}
        }
    }
}
