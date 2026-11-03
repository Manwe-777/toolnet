package io.manwe.toolnet

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
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
    private var netCallback: ConnectivityManager.NetworkCallback? = null

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
            Core.setExitNode(getSharedPreferences("toolnet", MODE_PRIVATE).getString("exit", "") ?: "")
            val err = Core.start(stateDir(this), deviceName())
            if (err.isNotEmpty()) Log.w(TAG, "start: $err")
            running = true
            generation = -1
            handler.post(poll)
            watchNetwork()
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
        // MagicDNS: all DNS goes to the core, which answers tailnet names and
        // forwards the rest (Android VPNs have no per-domain DNS)
        val dns = vpn.optString("dns")
        if (dns.isNotEmpty()) {
            b.addDnsServer(dns).addRoute(dns, 32)
            vpn.optString("domain").takeIf { it.isNotEmpty() }?.let { b.addSearchDomain(it) }
        }
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

    /**
     * Follows the real (non-VPN) network: on any change, tells the core and
     * hands it the active network's DNS servers. The default network callback
     * alone isn't reliable here (it didn't fire on Wi-Fi -> LTE with our VPN up).
     */
    private fun watchNetwork() {
        val cm = getSystemService(ConnectivityManager::class.java)
        var last: Network? = null
        var lastDns = ""
        fun refresh(reason: String) {
            val active = cm.activeNetwork // ours bypasses the VPN: the real one
            val caps = active?.let { cm.getNetworkCapabilities(it) }
            if (active == null || caps?.hasTransport(android.net.NetworkCapabilities.TRANSPORT_VPN) == true) return
            val dns = cm.getLinkProperties(active)?.dnsServers?.joinToString(",") { it.hostAddress ?: "" } ?: ""
            if (dns != lastDns) {
                lastDns = dns
                Core.setUpstreamDns(dns)
            }
            if (active != last) {
                last = active
                Log.i(TAG, "network: $active ($reason)")
                setUnderlyingNetworks(arrayOf(active))
                Core.networkChanged()
            }
        }
        val request = android.net.NetworkRequest.Builder()
            .addCapability(android.net.NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .removeCapability(android.net.NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
            .build()
        // Right after a loss there may be no active network yet: look again soon
        fun soon(reason: String) {
            for (delay in longArrayOf(0, 1000, 3000)) handler.postDelayed({ refresh(reason) }, delay)
        }
        val cb = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = soon("available")
            override fun onLost(network: Network) = soon("lost")
            override fun onLinkPropertiesChanged(network: Network, lp: LinkProperties) = soon("link")
        }
        cm.registerNetworkCallback(request, cb)
        netCallback = cb
        handler.post { refresh("start") }
    }

    private fun stopAll() {
        netCallback?.let {
            try {
                getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(it)
            } catch (_: Exception) {
            }
        }
        netCallback = null
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
