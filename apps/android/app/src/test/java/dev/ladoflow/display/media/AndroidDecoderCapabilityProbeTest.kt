package dev.ladoflow.display.media

import dev.ladoflow.display.protocol.InputCapabilities
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidDecoderCapabilityProbeTest {
    @Test
    fun advertisesEveryImplementedInputFamily() {
        assertTrue(androidDisplayInputCapabilities.contains(InputCapabilities.Pointer))
        assertTrue(androidDisplayInputCapabilities.contains(InputCapabilities.Touch))
        assertTrue(androidDisplayInputCapabilities.contains(InputCapabilities.Keyboard))
        assertEquals(
            InputCapabilities.Pointer or InputCapabilities.Touch or InputCapabilities.Keyboard,
            androidDisplayInputCapabilities,
        )
    }
}
