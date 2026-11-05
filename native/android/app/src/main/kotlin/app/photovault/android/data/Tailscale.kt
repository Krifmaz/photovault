package app.photovault.android.data

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.util.Log
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import java.net.InetAddress

/**
 * Tailscale's address ranges: 100.64.0.0/10 (IPv4, the shared CGNAT space it hands out from) and
 * fd7a:115c:a1e0::/48 (IPv6). Pure, so it is unit-tested.
 */
object TailscaleAddresses {
    private val IPV6_PREFIX = byteArrayOf(0xfd.toByte(), 0x7a, 0x11, 0x5c, 0xa1.toByte(), 0xe0.toByte())

    /** Whether the raw address bytes (4 for IPv4, 16 for IPv6) are in one of Tailscale's ranges. */
    fun contains(address: ByteArray): Boolean = when (address.size) {
        4 -> (address[0].toInt() and 0xFF) == 100 && (address[1].toInt() and 0xC0) == 0x40
        16 -> IPV6_PREFIX.indices.all { address[it] == IPV6_PREFIX[it] }
        else -> false
    }

    fun contains(address: InetAddress): Boolean = contains(address.address)

    /** Whether a VPN network with these interface addresses is Tailscale's. */
    fun isTailscaleVpn(addresses: Collection<InetAddress>): Boolean = addresses.any { contains(it) }
}

/** Whether Tailscale is up on this device, updated while collected. */
fun interface TailscaleStatus {
    fun active(): Flow<Boolean>

    companion object {
        /** Never active (tests, previews). */
        val Off = TailscaleStatus { kotlinx.coroutines.flow.flowOf(false) }
    }
}

/**
 * Tailscale detection with [ConnectivityManager]: a VPN-transport network whose link addresses include one in
 * [TailscaleAddresses]. Only VPNs that route this app's traffic are visible, which are the ones that matter here.
 */
class AndroidTailscaleStatus(context: Context) : TailscaleStatus {
    private val cm: ConnectivityManager? = context.applicationContext.getSystemService(ConnectivityManager::class.java)

    override fun active(): Flow<Boolean> = callbackFlow {
        val cm = cm
        if (cm == null) {
            trySend(false)
            awaitClose { }
            return@callbackFlow
        }
        val vpns = HashMap<Network, Boolean>()
        val lock = Any()

        fun update(network: Network, linkProperties: LinkProperties?) {
            val tailscale = linkProperties?.linkAddresses?.map { it.address }?.let(TailscaleAddresses::isTailscaleVpn) ?: false
            val any = synchronized(lock) {
                vpns[network] = tailscale
                vpns.values.any { it }
            }
            trySend(any)
        }

        val callback = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(network: Network) = update(network, runCatching { cm.getLinkProperties(network) }.getOrNull())
            override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) = update(network, linkProperties)
            override fun onLost(network: Network) {
                val any = synchronized(lock) {
                    vpns.remove(network)
                    vpns.values.any { it }
                }
                trySend(any)
            }
        }
        trySend(false)
        val registered = try {
            cm.registerNetworkCallback(vpnRequest(), callback)
            true
        } catch (e: RuntimeException) {
            // SecurityException without ACCESS_NETWORK_STATE, or too many callbacks: report "not active".
            Log.w(TAG, "VPN networks are not observed", e)
            false
        }
        awaitClose { if (registered) runCatching { cm.unregisterNetworkCallback(callback) } }
    }.distinctUntilChanged()

    companion object {
        private const val TAG = "PhotovaultTailscale"

        /** VPN networks (they lack NOT_VPN, which a default request asks for). */
        fun vpnRequest(): NetworkRequest = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_VPN)
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_NOT_VPN)
            .build()
    }
}
