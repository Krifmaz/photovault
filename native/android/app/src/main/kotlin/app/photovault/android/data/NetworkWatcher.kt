package app.photovault.android.data

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Calls [onChange] (core's `refresh_network`, contracts/server-info.md: "re-check on network change") when
 * the default network appears, goes away or changes its addresses or capabilities, debounced so a Wi-Fi
 * hand-over triggers one re-check. The callback runs on ConnectivityManager's thread; the work runs in
 * [scope] (IO).
 */
class NetworkWatcher(
    private val context: Context,
    private val scope: CoroutineScope,
    private val onChange: suspend () -> Unit,
) {
    private val lock = Any()
    private var pending: Job? = null

    fun start() {
        val cm = context.getSystemService(ConnectivityManager::class.java) ?: return
        try {
            cm.registerDefaultNetworkCallback(object : ConnectivityManager.NetworkCallback() {
                override fun onAvailable(network: Network) = changed()
                override fun onLost(network: Network) = changed()
                override fun onLinkPropertiesChanged(network: Network, linkProperties: LinkProperties) = changed()

                // Capability updates (signal strength and the like) can be frequent; the debounce folds them.
                override fun onCapabilitiesChanged(network: Network, networkCapabilities: NetworkCapabilities) = changed()
            })
        } catch (e: RuntimeException) {
            // SecurityException without ACCESS_NETWORK_STATE, or too many callbacks: keep the current choice.
            Log.w(TAG, "network changes are not observed", e)
        }
    }

    private fun changed() {
        synchronized(lock) {
            pending?.cancel()
            pending = scope.launch {
                delay(DEBOUNCE_MS)
                runCatching { onChange() }
            }
        }
    }

    private companion object {
        const val TAG = "PhotovaultNetwork"
        const val DEBOUNCE_MS = 1_500L
    }
}
