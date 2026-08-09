package dev.ladoflow.display.media

import android.view.Surface
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.unit.dp
import dev.ladoflow.display.protocol.DisplayConfigPayload
import dev.ladoflow.display.protocol.LdflFrame
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test

class MediaCodecSurfaceLifecycleTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun surfaceRebuildAndDirectionChangeRetainGenerationSafety() {
        val decoder = RecordingDecoder()
        val controller = DecoderSurfaceController(decoder)
        var showSurface by mutableStateOf(true)
        var landscape by mutableStateOf(false)

        composeRule.setContent {
            if (showSurface) {
                MediaCodecSurface(
                    surfaceController = controller,
                    modifier = if (landscape) {
                        Modifier.size(width = 360.dp, height = 200.dp)
                    } else {
                        Modifier.size(width = 200.dp, height = 360.dp)
                    },
                )
            }
        }

        composeRule.waitUntil(timeoutMillis = 5_000) {
            controller.state.value.let { state ->
                state.attached && (state.width ?: 0) < (state.height ?: 0)
            }
        }
        val firstGeneration = controller.state.value.generation
        assertTrue(decoder.surfaces.last() != null)

        composeRule.runOnUiThread { landscape = true }
        composeRule.waitUntil(timeoutMillis = 5_000) {
            controller.state.value.let { state ->
                state.attached && (state.width ?: 0) > (state.height ?: 0)
            }
        }
        assertEquals(firstGeneration, controller.state.value.generation)

        composeRule.runOnUiThread { showSurface = false }
        composeRule.waitUntil(timeoutMillis = 5_000) { !controller.state.value.attached }
        assertNull(decoder.surfaces.last())

        composeRule.runOnUiThread { showSurface = true }
        composeRule.waitUntil(timeoutMillis = 5_000) {
            controller.state.value.attached &&
                controller.state.value.generation > firstGeneration
        }
        assertTrue(decoder.surfaces.last() != null)
        assertFalse(controller.state.value.width == null || controller.state.value.height == null)
    }

    private class RecordingDecoder : VideoDecoder {
        private val mutableState = MutableStateFlow<VideoDecoderState>(VideoDecoderState.Idle)
        private val mutableEvents = MutableSharedFlow<VideoDecoderEvent>()
        private val mutableQueueDepth = MutableStateFlow(0)
        val surfaces = mutableListOf<Surface?>()

        override val state: StateFlow<VideoDecoderState> = mutableState.asStateFlow()
        override val events: SharedFlow<VideoDecoderEvent> = mutableEvents.asSharedFlow()
        override val queueDepth: StateFlow<Int> = mutableQueueDepth.asStateFlow()

        override fun setOutputSurface(surface: Surface?) {
            surfaces += surface
        }

        override fun applyConfiguration(configuration: DisplayConfigPayload) = Unit

        override fun submit(frame: LdflFrame): Boolean = true

        override fun reset(reason: String) = Unit

        override fun close() {
            mutableState.value = VideoDecoderState.Closed
        }
    }
}
