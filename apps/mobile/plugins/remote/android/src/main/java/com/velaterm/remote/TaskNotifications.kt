package com.velaterm.remote

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.ComponentActivity
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import org.json.JSONObject

/** Local foreground fallback until the host acknowledges a background push subscription. */
internal class TaskNotifications(private val activity: ComponentActivity) {
    private var waiting: ((String) -> Unit)? = null
    private val preferences = activity.getSharedPreferences("vela-notifications", Context.MODE_PRIVATE)
    private val launcher = activity.activityResultRegistry.register("vela-notifications", ActivityResultContracts.RequestPermission()) { granted ->
        val callback = waiting; waiting = null; callback?.invoke(if (granted && enabled()) "granted" else "denied")
    }
    private fun enabled() = NotificationManagerCompat.from(activity).areNotificationsEnabled()
    fun permission(request: Boolean, completion: (String) -> Unit) {
        if (Build.VERSION.SDK_INT < 33 || ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) {
            completion(if (enabled()) "granted" else "denied"); return
        }
        if (!request) { completion(if (preferences.getBoolean("requested", false)) "denied" else "default"); return }
        if (waiting != null) { completion("default"); return }
        waiting = completion; preferences.edit().putBoolean("requested", true).apply()
        launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
    }
    fun send(connectionId: String, body: JSONObject) {
        val title = body.getString("title"); val text = body.getString("body"); val session = body.optString("sessionId", "")
        require(title.isNotEmpty() && title.toByteArray().size <= 1024 && text.toByteArray().size <= 4096 && session.toByteArray().size <= 256) { "Invalid notification" }
        check(enabled()) { "Notifications are not permitted" }
        val sound = body.optBoolean("sound", false)
        val channelId = if (sound) "task-updates" else "task-updates-silent"
        if (Build.VERSION.SDK_INT >= 26) {
            val channel = NotificationChannel(channelId, if (sound) "Task updates" else "Task updates (silent)", if (sound) NotificationManager.IMPORTANCE_HIGH else NotificationManager.IMPORTANCE_DEFAULT)
            if (!sound) { channel.setSound(null, null); channel.enableVibration(false) }
            activity.getSystemService(NotificationManager::class.java).createNotificationChannel(channel)
        }
        val intent = activity.packageManager.getLaunchIntentForPackage(activity.packageName) ?: error("App unavailable")
        intent.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
        intent.putExtra("velaNotificationConnection", connectionId); intent.putExtra("velaNotificationSession", session)
        intent.data = android.net.Uri.Builder().scheme("velaterm-notification").authority("session").appendPath(connectionId).appendPath(session).build()
        val pending = PendingIntent.getActivity(activity, 0, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification = NotificationCompat.Builder(activity, channelId)
            .setSmallIcon(R.drawable.push_small).setContentTitle(title).setContentText(text)
            .setStyle(NotificationCompat.BigTextStyle().bigText(text)).setAutoCancel(true).setContentIntent(pending)
            .setVisibility(NotificationCompat.VISIBILITY_PRIVATE).setPriority(NotificationCompat.PRIORITY_HIGH)
        if (sound) notification.setDefaults(android.app.Notification.DEFAULT_SOUND)
        else notification.setSilent(true)
        NotificationManagerCompat.from(activity).notify(connectionId + ":" + session, 1, notification.build())
    }
    fun close() { launcher.unregister(); waiting?.invoke("unsupported"); waiting = null }
}
