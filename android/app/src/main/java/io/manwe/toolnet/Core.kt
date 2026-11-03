package io.manwe.toolnet

/**
 * toolnet's Rust core (android/rust), over JNI. No callbacks: the service
 * polls [status], builds the VPN the core asks for, and hands its fd back
 * with [setTun].
 */
object Core {
    init {
        System.loadLibrary("toolnet_android")
    }

    /** Creates the device identity if needed; returns the join code, or "error: ..." */
    external fun setup(dir: String, owner: String, net: String, name: String): String

    /** The join code for the owner app, or "" if not set up */
    external fun joinCode(dir: String, name: String): String

    /** QR code of [text]: rows of '1' (dark) / '0', separated by newlines */
    external fun qr(text: String): String

    /** Starts the node; "" or an error message */
    external fun start(dir: String, name: String): String

    /** Gives the core the VPN's TUN fd (detached, the core closes it) */
    external fun setTun(fd: Int, generation: Int): Boolean

    /** JSON: running, status, member, error, vpn {generation, ip, prefix, routes}, peers */
    external fun status(): String

    external fun stop()

    /** The underlying network's DNS servers, comma separated (for forwarding) */
    external fun setUpstreamDns(servers: String)

    /** The default network changed: makes iroh rebind and re-punch now */
    external fun networkChanged()

    /** Uses this member as exit node ("" for none) */
    external fun setExitNode(name: String)
}
