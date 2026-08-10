package dev.ladoflow.display.transport.usb

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class UsbTransportContractTest {
    @Test
    fun `accessory matcher requires the published AOA identity`() {
        val matching = UsbAccessoryIdentity(
            manufacturer = "LadoFlow",
            model = "LadoFlow Host",
            description = "Office PC",
            version = "1.0",
            serial = "host-1",
        )

        assertTrue(matching.isLadoFlowHost)
        assertEquals("Office PC", matching.displayName)
        assertFalse(matching.copy(manufacturer = "Other").isLadoFlowHost)
        assertFalse(matching.copy(model = "Other").isLadoFlowHost)
    }

    @Test
    fun `reconnect policy is bounded exponential backoff`() {
        val policy = UsbReconnectPolicy(
            initialDelayMillis = 250,
            maximumDelayMillis = 2_000,
            maximumAttempts = 5,
        )

        assertEquals(listOf(250L, 500L, 1_000L, 2_000L, 2_000L), (1..5).map(policy::delayMillis))
        assertTrue(policy.shouldRetry(5))
        assertFalse(policy.shouldRetry(6))
    }

    @Test
    fun `matching physical detach is explicit only while product transport is active`() {
        val accessory = UsbAccessoryIdentity(
            manufacturer = "LadoFlow",
            model = "LadoFlow Host",
            description = "Office PC",
            version = "1.0",
            serial = null,
        )

        assertEquals(
            UsbTransportState.Detached(accessory),
            resolveUsbDetachState(
                current = accessory,
                detached = accessory,
                stillAttached = false,
                foreground = true,
                userPaused = false,
            ),
        )
        assertEquals(
            UsbTransportState.Stopped,
            resolveUsbDetachState(
                current = accessory,
                detached = accessory,
                stillAttached = false,
                foreground = false,
                userPaused = false,
            ),
        )
        assertNull(
            resolveUsbDetachState(
                current = accessory,
                detached = accessory.copy(description = "Other PC"),
                stillAttached = false,
                foreground = true,
                userPaused = false,
            ),
        )
        assertNull(
            resolveUsbDetachState(
                current = accessory,
                detached = accessory,
                stillAttached = true,
                foreground = true,
                userPaused = false,
            ),
        )
    }
}
