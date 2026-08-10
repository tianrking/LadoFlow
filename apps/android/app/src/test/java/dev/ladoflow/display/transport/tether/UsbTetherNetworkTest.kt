package dev.ladoflow.display.transport.tether

import java.net.InetAddress
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class UsbTetherNetworkTest {
    @Test
    fun `selector binds RNDIS private IPv4 and ignores WiFi`() {
        val selected = selectUsbTetherAddress(
            listOf(
                candidate("wlan0", "192.168.1.40"),
                candidate("rndis0", "192.168.42.129"),
            ),
        )

        assertEquals("192.168.42.129", selected?.hostAddress)
    }

    @Test
    fun `selector accepts NCM link local address as wired fallback`() {
        val selected = selectUsbTetherAddress(
            listOf(candidate("NCM0", "169.254.10.2")),
        )

        assertEquals("169.254.10.2", selected?.hostAddress)
    }

    @Test
    fun `selector rejects wildcard loopback public Ethernet VPN and down interfaces`() {
        val selected = selectUsbTetherAddress(
            listOf(
                candidate("rndis0", "0.0.0.0"),
                candidate("usb0", "127.0.0.1", isLoopback = true),
                candidate("usb1", "8.8.8.8"),
                candidate("eth0", "192.168.50.1"),
                candidate("tun0", "10.8.0.2"),
                candidate("ecm0", "192.168.44.1", isUp = false),
            ),
        )

        assertNull(selected)
    }

    @Test
    fun `selector uses stable interface preference without wildcard fallback`() {
        val selected = selectUsbTetherAddress(
            listOf(
                candidate("usb0", "192.168.44.1"),
                candidate("ncm0", "192.168.43.1"),
                candidate("rndis0", "192.168.42.1"),
            ),
        )

        assertEquals("192.168.42.1", selected?.hostAddress)
    }

    private fun candidate(
        name: String,
        address: String,
        isUp: Boolean = true,
        isLoopback: Boolean = false,
    ) = UsbTetherInterfaceCandidate(
        name = name,
        isUp = isUp,
        isLoopback = isLoopback,
        addresses = listOf(InetAddress.getByName(address)),
    )
}
