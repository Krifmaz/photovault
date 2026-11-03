package app.photovault.android.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ConnectQrTest {
    @Test
    fun parsesConnectUrl() {
        val p = ConnectQr.parse(
            "photovault://connect?url=https%3A%2F%2Fphotos.example.com%2F&home=http%3A%2F%2F10.0.1.7%3A8095&name=Home%20vault",
        )!!
        assertEquals("https://photos.example.com", p.url)
        assertEquals("http://10.0.1.7:8095", p.home)
        assertEquals("Home vault", p.name)
    }

    @Test
    fun fallsBackToHomeWithoutUrl() {
        assertEquals("http://10.0.1.7:8095", ConnectQr.address("photovault://connect?home=http%3A%2F%2F10.0.1.7%3A8095"))
    }

    @Test
    fun acceptsBareUrls() {
        assertEquals("https://photos.example.com", ConnectQr.address("  https://photos.example.com/ "))
        assertEquals("http://10.0.1.7:8095", ConnectQr.address("http://10.0.1.7:8095"))
    }

    @Test
    fun rejectsOtherPayloads() {
        assertNull(ConnectQr.address(""))
        assertNull(ConnectQr.address("WIFI:S:home;T:WPA;P:secret;;"))
        assertNull(ConnectQr.address("photovault://connect?name=x"))
        assertNull(ConnectQr.address("photovault://connect?url=javascript%3Aalert(1)"))
        assertNull(ConnectQr.address("https://"))
    }

    @Test
    fun displayAddressDropsScheme() {
        assertEquals("10.0.1.7:8095", displayAddress("http://10.0.1.7:8095/"))
    }
}
