package app.photovault.android.data

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.net.InetAddress

class TailscaleAddressesTest {
    // Literals only: InetAddress.getByName does no lookup for them.
    private fun ip(literal: String): InetAddress = InetAddress.getByName(literal)

    @Test
    fun ipv4RangeIs100_64Slash10() {
        assertTrue(TailscaleAddresses.contains(ip("100.101.102.103")))
        assertTrue(TailscaleAddresses.contains(ip("100.64.0.0")))
        assertTrue(TailscaleAddresses.contains(ip("100.127.255.255")))
        assertFalse(TailscaleAddresses.contains(ip("100.63.255.255")))
        assertFalse(TailscaleAddresses.contains(ip("100.128.0.0")))
        assertFalse(TailscaleAddresses.contains(ip("192.168.1.10")))
        assertFalse(TailscaleAddresses.contains(ip("10.0.0.1")))
        assertFalse(TailscaleAddresses.contains(ip("101.101.102.103")))
    }

    @Test
    fun ipv6RangeIsFd7a115cA1e0Slash48() {
        assertTrue(TailscaleAddresses.contains(ip("fd7a:115c:a1e0::1")))
        assertTrue(TailscaleAddresses.contains(ip("fd7a:115c:a1e0:ab12:4843:cd96:6265:6667")))
        assertFalse(TailscaleAddresses.contains(ip("fd7a:115c:a1e1::1")))
        assertFalse(TailscaleAddresses.contains(ip("fd00::1")))
        assertFalse(TailscaleAddresses.contains(ip("2001:db8::1")))
        assertFalse(TailscaleAddresses.contains(ip("::1")))
    }

    @Test
    fun oddLengthsAreNotTailscale() {
        assertFalse(TailscaleAddresses.contains(ByteArray(0)))
        assertFalse(TailscaleAddresses.contains(byteArrayOf(100, 64, 0)))
    }

    @Test
    fun aVpnIsTailscaleWhenAnyOfItsAddressesIs() {
        assertTrue(TailscaleAddresses.isTailscaleVpn(listOf(ip("fe80::1"), ip("100.101.102.103"))))
        assertFalse(TailscaleAddresses.isTailscaleVpn(listOf(ip("10.8.0.2"))))
        assertFalse(TailscaleAddresses.isTailscaleVpn(emptyList()))
    }
}
