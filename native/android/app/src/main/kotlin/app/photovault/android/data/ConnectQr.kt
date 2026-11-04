package app.photovault.android.data

import java.net.URI
import java.net.URLDecoder

/**
 * The connect QR payload of contracts/server-info.md:
 * `photovault://connect?url=<publicUrl>&home=<homeUrl>&name=<name>`, or a bare http(s) URL.
 *
 * The real app uses core's `parse_connect_qr`; this Kotlin version backs FakeCore and the unit tests and
 * follows the same rules: the server address is `url` (falling back to `home`), with no trailing slash.
 */
object ConnectQr {
    data class Payload(val url: String, val home: String?, val name: String?)

    fun parse(payload: String): Payload? {
        val text = payload.trim()
        if (text.isEmpty()) return null
        val lower = text.lowercase()
        if (lower.startsWith("http://") || lower.startsWith("https://")) {
            val uri = runCatching { URI(text) }.getOrNull() ?: return null
            if (uri.host.isNullOrBlank()) return null
            return Payload(text.trimEnd('/'), null, null)
        }
        if (!lower.startsWith("photovault://connect")) return null
        val query = runCatching { URI(text).rawQuery }.getOrNull() ?: return null
        val params = query.split('&').mapNotNull { part ->
            val eq = part.indexOf('=')
            if (eq <= 0) null else part.substring(0, eq) to decode(part.substring(eq + 1))
        }.toMap()
        val url = params["url"]?.takeIf { it.isNotBlank() }?.trimEnd('/')
        val home = params["home"]?.takeIf { it.isNotBlank() }?.trimEnd('/')
        val address = url ?: home ?: return null
        if (!isHttpUrl(address)) return null
        return Payload(address, home, params["name"]?.takeIf { it.isNotBlank() })
    }

    /** The address to connect to, as core's `parse_connect_qr` returns it. */
    fun address(payload: String): String? = parse(payload)?.url

    private fun isHttpUrl(s: String): Boolean {
        val l = s.lowercase()
        return (l.startsWith("http://") || l.startsWith("https://")) && runCatching { URI(s).host }.getOrNull() != null
    }

    private fun decode(s: String): String = runCatching { URLDecoder.decode(s, "UTF-8") }.getOrDefault(s)
}

/** What the user typed, as shown in messages: no scheme, no trailing slash ("10.0.1.7:8095"). */
fun displayAddress(address: String): String =
    address.trim().removePrefix("https://").removePrefix("http://").trimEnd('/')
