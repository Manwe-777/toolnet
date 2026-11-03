package io.manwe.toolnet

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Log
import org.json.JSONObject

/**
 * Runs the core and keeps the VPN in the shape it asks for: the tailnet
 * prefix plus approved subnet routes. The app's own sockets (iroh, Nostr)
 * stay outside the VPN, so the tunnel never carries itself.
 */
class ToolnetVpnService : VpnService() {
    private val handler = Handler(Looper.getMainLooper())
    private var generation = -1
    private var running = false

    private val poll = object : Runnable {
        override fun run() {
            if (!running) return
            try {
                apply(JSONObject(Core.status()))
            } catch (e: Exception) {
                Log.w(TAG, "status: $e")
            }
            handler.postDelayed(this, 1000)
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            stopAll()
            return START_NOT_STICKY
        }
        startForegroundCompat()
        if (!running) {
            val err = Core.start(stateDir(this), deviceName())
            if (err.isNotEmpty()) Log.w(TAG, "start: $err")
            running = true
            generation = -1
            handler.post(poll)
        }
        return START_STICKY
    }

    private fun apply(s: JSONObject) {
        val vpn = s.optJSONObject("vpn") ?: return
        val gen = vpn.getInt("generation")
        if (gen == generation) return
        val ip = vpn.getString("ip")
        val prefix = vpn.getInt("prefix")
        val b = Builder()
            .setSession("toolnet")
            .setMtu(1150)
            .addAddress(ip, prefix)
            .addRoute(network(ip, prefix), prefix)
        val routes = vpn.getJSONArray("routes")
        for (i in 0 until routes.length()) {
            val (addr, len) = routes.getString(i).split("/")
            b.addRoute(addr, len.toInt())
        }
        try {
            b.addDisallowedApplication(packageName)
        } catch (e: Exception) {
            Log.w(TAG, "can't exclude ourselves from the VPN: $e")
        }
        val pfd = b.establish()
        if (pfd == null) {
            Log.w(TAG, "VPN not prepared (permission revoked?)")
            return
        }
        Core.setTun(pfd.detachFd(), gen)
        generation = gen
        Log.i(TAG, "VPN $ip/$prefix routes=$routes (generation $gen)")
    }

    private fun stopAll() {
        running = false
        handler.removeCallbacks(poll)
        Thread { Core.stop() }.start()
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onRevoke() {
        stopAll()
    }

    override fun onDestroy() {
        if (running) stopAll()
        super.onDestroy()
    }

    private fun startForegroundCompat() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "toolnet", NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val stop = PendingIntent.getService(
            this, 1, Intent(this, ToolnetVpnService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE
        )
        val n = Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_toolnet)
            .setContentTitle("toolnet")
            .setContentText("Connected to your network")
            .setContentIntent(open)
            .addAction(Notification.Action.Builder(null, "Disconnect", stop).build())
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED)
        } else {
            startForeground(1, n)
        }
    }

    companion object {
        const val TAG = "toolnet"
        const val ACTION_STOP = "io.manwe.toolnet.STOP"
        private const val CHANNEL = "toolnet"

        fun stateDir(c: Context): String = c.filesDir.resolve("state").absolutePath

        fun deviceName(): String =
            Build.MODEL.lowercase().replace(Regex("[^a-z0-9]+"), "-").trim('-').ifEmpty { "phone" }

        /** The network address of ip/prefix ("10.213.0.7", 24 -> "10.213.0.0") */
        fun network(ip: String, prefix: Int): String {
            val p = ip.split(".").map { it.toInt() }
            val v = (p[0] shl 24) or (p[1] shl 16) or (p[2] shl 8) or p[3]
            val mask = if (prefix == 0) 0 else (-1 shl (32 - prefix))
            val n = v and mask
            return "${(n ushr 24) and 255}.${(n ushr 16) and 255}.${(n ushr 8) and 255}.${n and 255}"
        }
    }
}
