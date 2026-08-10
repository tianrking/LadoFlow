package dev.ladoflow.display.media

import dev.ladoflow.display.protocol.InputCapabilities
import dev.ladoflow.display.protocol.FeatureFlags
import org.junit.Assert.assertFalse
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

    @Test
    fun advertisesNoUnimplementedFeatureAndMatchesTheHostStandardModeContract() {
        assertEquals(FeatureFlags.None, androidDisplayFeatureCapabilities)
        assertFalse(androidDisplayFeatureCapabilities.contains(FeatureFlags.DynamicRotation))
        assertEquals(
            listOf(
                2732 to 2048,
                2560 to 1600,
                2560 to 1440,
                2048 to 1536,
                1920 to 1200,
                1920 to 1080,
                1600 to 1200,
                1366 to 768,
                1280 to 800,
                1280 to 720,
                1024 to 768,
                1024 to 640,
                960 to 600,
                960 to 540,
                800 to 600,
                800 to 500,
                640 to 480,
                640 to 400,
            ),
            coordinatedHostDisplayModes.map { it.width to it.height },
        )
        assertTrue(coordinatedHostDisplayModes.all { it.width % 2 == 0 && it.height % 2 == 0 })
        assertEquals(coordinatedHostDisplayModes.size, coordinatedHostDisplayModes.distinct().size)
    }
}
