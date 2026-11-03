package io.manwe.toolnet

import android.Manifest
import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.net.VpnService
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject

/**
 * The only screen. Not set up: owner + network form. Set up: this phone's join
 * code (QR + copy, for the owner app), the connect switch, and status.
 */
class MainActivity : Activity() {
    private val ui = Handler(Looper.getMainLooper())
    private lateinit var root: LinearLayout
    private var statusText: TextView? = null
    private var peersText: TextView? = null
    private var connect: Switch? = null
    private var updating = false

    private val dark = Color.parseColor("#0d1117")
    private val card = Color.parseColor("#161b22")
    private val fg = Color.parseColor("#e6edf3")
    private val muted = Color.parseColor("#8b949e")
    private val green = Color.parseColor("#3fb950")

    private val refresh = object : Runnable {
        override fun run() {
            updateStatus()
            ui.postDelayed(this, 1000)
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(24), dp(16), dp(24))
        }
        setContentView(ScrollView(this).apply {
            setBackgroundColor(dark)
            addView(root)
        })
        if (Build.VERSION.SDK_INT >= 33) requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        render()
        handleConnectExtra(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleConnectExtra(intent)
    }

    /** `am start ... --ez connect true|false`: connect without touching the
     * screen (testing over adb; the VPN consent must have been given once) */
    private fun handleConnectExtra(intent: Intent?) {
        if (intent?.hasExtra("connect") == true && Core.joinCode(stateDir(), name()).isNotEmpty()) {
            toggle(intent.getBooleanExtra("connect", false))
        }
    }

    override fun onResume() {
        super.onResume()
        ui.post(refresh)
    }

    override fun onPause() {
        ui.removeCallbacks(refresh)
        super.onPause()
    }

    private fun stateDir() = ToolnetVpnService.stateDir(this)
    private fun name() = getSharedPreferences("toolnet", MODE_PRIVATE).getString("name", null) ?: ToolnetVpnService.deviceName()

    private fun render() {
        root.removeAllViews()
        statusText = null; peersText = null; connect = null
        root.addView(label("toolnet", 26f, fg, bold = true))
        root.addView(label("Your private network, over tool-db and iroh", 14f, muted).also { it.setPadding(0, 0, 0, dp(16)) })
        val code = Core.joinCode(stateDir(), name())
        if (code.isEmpty()) renderSetup() else renderMain(code)
    }

    private fun renderSetup() {
        val box = cardView()
        box.addView(label("Join a network", 18f, fg, bold = true))
        box.addView(label("The owner npub is in the owner app's settings.", 14f, muted))
        val owner = input("Owner npub1…")
        val net = input("Network name (e.g. home)")
        val name = input("This phone's name").apply { setText(ToolnetVpnService.deviceName()) }
        box.addView(owner); box.addView(net); box.addView(name)
        box.addView(button("Set up") {
            val result = Core.setup(stateDir(), owner.text.toString(), net.text.toString(), name.text.toString())
            if (result.startsWith("error:")) {
                toast(result.removePrefix("error:").trim())
            } else {
                getSharedPreferences("toolnet", MODE_PRIVATE).edit().putString("name", name.text.toString().trim()).apply()
                render()
            }
        })
        root.addView(box)
    }

    private fun renderMain(code: String) {
        android.util.Log.i(ToolnetVpnService.TAG, "join code: $code")
        val status = cardView()
        val row = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER_VERTICAL }
        row.addView(label("Connected", 18f, fg, bold = true).apply {
            layoutParams = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
        })
        connect = Switch(this).apply {
            setOnCheckedChangeListener { _, on -> if (!updating) toggle(on) }
        }
        row.addView(connect)
        status.addView(row)
        statusText = label("", 14f, muted).also { status.addView(it) }
        peersText = label("", 14f, fg, mono = true).also { status.addView(it) }
        root.addView(status)

        val join = cardView()
        join.addView(label("Approve this phone", 18f, fg, bold = true))
        join.addView(label("In the owner app: Add a device, then scan this code from another screen, or paste it.", 14f, muted))
        join.addView(ImageView(this).apply {
            setImageBitmap(qrBitmap(code))
            adjustViewBounds = true
            setPadding(0, dp(12), 0, dp(12))
        })
        join.addView(button("Copy join code") {
            getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("toolnet join code", code))
            toast("Join code copied")
        })
        join.addView(label(code, 11f, muted, mono = true))
        root.addView(join)
    }

    private fun toggle(on: Boolean) {
        if (on) {
            val consent = VpnService.prepare(this)
            if (consent != null) startActivityForResult(consent, 2) else startVpn()
        } else {
            startService(Intent(this, ToolnetVpnService::class.java).setAction(ToolnetVpnService.ACTION_STOP))
        }
    }

    @Deprecated("Activity result API without AndroidX")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == 2) {
            if (resultCode == RESULT_OK) startVpn() else toast("VPN permission needed to connect")
        }
    }

    private fun startVpn() {
        startForegroundService(Intent(this, ToolnetVpnService::class.java))
    }

    private fun updateStatus() {
        val st = statusText ?: return
        val s = try { JSONObject(Core.status()) } catch (e: Exception) { return }
        val running = s.optBoolean("running")
        updating = true
        connect?.isChecked = running
        updating = false
        val vpn = s.optJSONObject("vpn")
        val lines = mutableListOf(s.optString("status"))
        if (vpn != null) lines += "This phone: ${vpn.optString("ip")}"
        s.optString("error").takeIf { it.isNotEmpty() && it != "null" }?.let { lines += "Error: $it" }
        st.text = lines.joinToString("\n")
        val peers = s.optJSONArray("peers")
        val out = StringBuilder()
        if (peers != null) for (i in 0 until peers.length()) {
            val p = peers.getJSONObject(i)
            val path = p.optString("path")
            val kind = when {
                path.startsWith("DIRECT") -> "direct"
                path.startsWith("RELAY") -> "relay"
                else -> path
            }
            val rtt = Regex("rtt ([0-9.]+)(ms|µs|s)").find(path)?.let { " ${it.groupValues[1].take(5)}${it.groupValues[2]}" } ?: ""
            out.append("${p.optString("name")}  ${p.optString("ip")}  $kind$rtt\n")
        }
        peersText?.text = out.toString().trimEnd()
    }

    private fun qrBitmap(code: String): Bitmap {
        val rows = Core.qr(code).split("\n").filter { it.isNotEmpty() }
        val n = rows.size
        val quiet = 4
        val scale = 10
        val size = (n + 2 * quiet) * scale
        val bmp = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        bmp.eraseColor(Color.WHITE)
        for (y in 0 until n) for (x in 0 until n) {
            if (rows[y][x] == '1') {
                for (dy in 0 until scale) for (dx in 0 until scale) {
                    bmp.setPixel((x + quiet) * scale + dx, (y + quiet) * scale + dy, Color.BLACK)
                }
            }
        }
        return bmp
    }

    // Small view helpers
    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    private fun label(s: String, size: Float, color: Int, bold: Boolean = false, mono: Boolean = false) =
        TextView(this).apply {
            text = s
            textSize = size
            setTextColor(color)
            typeface = when {
                mono -> Typeface.MONOSPACE
                bold -> Typeface.DEFAULT_BOLD
                else -> Typeface.DEFAULT
            }
            setPadding(0, dp(4), 0, dp(4))
        }

    private fun input(hint: String) = EditText(this).apply {
        this.hint = hint
        setTextColor(fg)
        setHintTextColor(muted)
        inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        isSingleLine = true
    }

    private fun button(s: String, onClick: () -> Unit) = Button(this).apply {
        text = s
        setTextColor(Color.WHITE)
        background = GradientDrawable().apply { setColor(green); cornerRadius = dp(8).toFloat() }
        setOnClickListener { onClick() }
        layoutParams = LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            topMargin = dp(8)
        }
    }

    private fun cardView() = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(16), dp(16), dp(16), dp(16))
        background = GradientDrawable().apply { setColor(card); cornerRadius = dp(12).toFloat() }
        layoutParams = LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply {
            bottomMargin = dp(12)
        }
    }

    private fun toast(s: String) = Toast.makeText(this, s, Toast.LENGTH_LONG).show()
}
