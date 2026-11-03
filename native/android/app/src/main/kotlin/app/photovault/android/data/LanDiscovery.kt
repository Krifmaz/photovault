package app.photovault.android.data

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.conflate
import java.net.Inet6Address

/** A server announced on the local network (contracts/apps.json "discovery"). */
data class DiscoveredServer(
    /** DNS-SD instance name, unique on the network; the list key. */
    val serviceName: String,
    val name: String,
    /** What to connect to: TXT `public` if announced, else http://host:port. */
    val address: String,
    val version: String?,
)

/** Photovault servers on this network, updated while collected. */
fun interface LanDiscovery {
    fun servers(): Flow<List<DiscoveredServer>>
}

/**
 * DNS-SD browsing for `_photovault._tcp` with Android's [NsdManager] (core's `discover_servers` is not used
 * on Android). Resolves run one at a time, as NsdManager requires before Android 14.
 */
class NsdLanDiscovery(context: Context) : LanDiscovery {
    private val nsd = context.applicationContext.getSystemService(NsdManager::class.java)

    override fun servers(): Flow<List<DiscoveredServer>> = callbackFlow {
        val found = LinkedHashMap<String, DiscoveredServer>()
        val queue = ArrayDeque<NsdServiceInfo>()
        var resolving = false
        val lock = Any()

        fun publish() {
            trySend(synchronized(lock) { found.values.toList() })
        }

        fun resolveNext() {
            val next = synchronized(lock) {
                if (resolving) return
                queue.removeFirstOrNull()?.also { resolving = true }
            } ?: return
            @Suppress("DEPRECATION")
            nsd.resolveService(next, object : NsdManager.ResolveListener {
                override fun onResolveFailed(serviceInfo: NsdServiceInfo, errorCode: Int) {
                    synchronized(lock) { resolving = false }
                    resolveNext()
                }

                override fun onServiceResolved(serviceInfo: NsdServiceInfo) {
                    val server = serviceInfo.toServer()
                    synchronized(lock) {
                        resolving = false
                        if (server != null) found[serviceInfo.serviceName] = server
                    }
                    publish()
                    resolveNext()
                }
            })
        }

        val listener = object : NsdManager.DiscoveryListener {
            override fun onDiscoveryStarted(serviceType: String) = Unit
            override fun onDiscoveryStopped(serviceType: String) = Unit
            override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) = Unit
            override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) = Unit

            override fun onServiceFound(serviceInfo: NsdServiceInfo) {
                synchronized(lock) { queue.addLast(serviceInfo) }
                resolveNext()
            }

            override fun onServiceLost(serviceInfo: NsdServiceInfo) {
                val removed = synchronized(lock) { found.remove(serviceInfo.serviceName) != null }
                if (removed) publish()
            }
        }

        publish()
        val started = runCatching { nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener) }.isSuccess
        awaitClose {
            if (started) runCatching { nsd.stopServiceDiscovery(listener) }
        }
    }.conflate()

    private fun NsdServiceInfo.toServer(): DiscoveredServer? {
        val txt = attributes.mapValues { (_, v) -> v?.toString(Charsets.UTF_8).orEmpty() }
        @Suppress("DEPRECATION")
        val hostAddress = host?.let { if (it is Inet6Address) "[${it.hostAddress}]" else it.hostAddress }
        val address = txt["public"]?.takeIf { it.isNotBlank() }
            ?: hostAddress?.let { "http://$it:$port" }
            ?: return null
        return DiscoveredServer(
            serviceName = serviceName,
            name = txt["name"]?.takeIf { it.isNotBlank() } ?: serviceName,
            address = address,
            version = txt["version"]?.takeIf { it.isNotBlank() },
        )
    }

    companion object {
        const val SERVICE_TYPE = "_photovault._tcp"
    }
}
