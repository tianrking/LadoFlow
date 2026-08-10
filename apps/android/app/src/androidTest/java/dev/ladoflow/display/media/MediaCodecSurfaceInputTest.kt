package dev.ladoflow.display.media

import android.view.KeyEvent
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import dev.ladoflow.display.input.AndroidInputController
import dev.ladoflow.display.input.AndroidInputEmission
import dev.ladoflow.display.protocol.ButtonState
import dev.ladoflow.display.protocol.KeyInput
import dev.ladoflow.display.protocol.KeyModifiers
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MediaCodecSurfaceInputTest {
    @Test
    fun surfaceViewForwardsPhysicalKeyDownAndUpToInputController() {
        val emissions = CopyOnWriteArrayList<AndroidInputEmission>()
        val controller = AndroidInputController(emissions::add)
        val instrumentation = InstrumentationRegistry.getInstrumentation()

        instrumentation.runOnMainSync {
            val surfaceView = DecoderSurfaceView(instrumentation.targetContext)
            surfaceView.installInputController(controller)

            assertTrue(surfaceView.isFocusable)
            assertTrue(surfaceView.isFocusableInTouchMode)
            assertTrue(
                surfaceView.dispatchKeyEvent(
                    KeyEvent(
                        10L,
                        10L,
                        KeyEvent.ACTION_DOWN,
                        KeyEvent.KEYCODE_A,
                        0,
                        KeyEvent.META_SHIFT_ON,
                    ),
                ),
            )
            assertTrue(
                surfaceView.dispatchKeyEvent(
                    KeyEvent(
                        10L,
                        20L,
                        KeyEvent.ACTION_UP,
                        KeyEvent.KEYCODE_A,
                        0,
                        KeyEvent.META_SHIFT_ON,
                    ),
                ),
            )
        }

        val keys = emissions.mapNotNull { it.payload.event as? KeyInput }
        assertEquals(listOf(ButtonState.Pressed, ButtonState.Released), keys.map(KeyInput::state))
        assertTrue(keys.all { it.usage == 0x04 })
        assertTrue(keys.all { it.modifiers.contains(KeyModifiers.Shift) })
    }
}
