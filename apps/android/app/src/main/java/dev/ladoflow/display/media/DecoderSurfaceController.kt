package dev.ladoflow.display.media

import android.view.Surface
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

@JvmInline
value class DecoderSurfaceLease internal constructor(val generation: Long)

data class DecoderSurfaceState(
    val generation: Long = 0,
    val attached: Boolean = false,
    val width: Int? = null,
    val height: Int? = null,
)

/**
 * Owns the decoder Surface across Activity/Compose recreation. A stale view may
 * release only its own lease, so it cannot detach a newer Surface.
 */
class DecoderSurfaceController internal constructor(decoder: VideoDecoder) {
    private val coordinator = SurfaceLeaseCoordinator<Surface>(decoder::setOutputSurface)

    val state: StateFlow<DecoderSurfaceState> = coordinator.state

    internal fun attach(surface: Surface): DecoderSurfaceLease = coordinator.attach(surface)

    internal fun resize(
        lease: DecoderSurfaceLease,
        width: Int,
        height: Int,
    ): Boolean = coordinator.resize(lease, width, height)

    internal fun detach(lease: DecoderSurfaceLease): Boolean = coordinator.detach(lease)

    internal fun release(): Boolean = coordinator.release()
}

internal class SurfaceLeaseCoordinator<T : Any>(
    private val updateSurface: (T?) -> Unit,
) {
    private val mutableState = MutableStateFlow(DecoderSurfaceState())
    private var activeLease: DecoderSurfaceLease? = null
    private var generation = 0L

    val state: StateFlow<DecoderSurfaceState> = mutableState.asStateFlow()

    @Synchronized
    fun attach(surface: T): DecoderSurfaceLease {
        generation = if (generation == Long.MAX_VALUE) 1L else generation + 1L
        val lease = DecoderSurfaceLease(generation)
        activeLease = lease
        mutableState.value = DecoderSurfaceState(generation = generation, attached = true)
        updateSurface(surface)
        return lease
    }

    @Synchronized
    fun resize(
        lease: DecoderSurfaceLease,
        width: Int,
        height: Int,
    ): Boolean {
        if (lease != activeLease || width <= 0 || height <= 0) return false
        mutableState.value = mutableState.value.copy(width = width, height = height)
        return true
    }

    @Synchronized
    fun detach(lease: DecoderSurfaceLease): Boolean {
        if (lease != activeLease) return false
        activeLease = null
        mutableState.value = DecoderSurfaceState(generation = generation)
        updateSurface(null)
        return true
    }

    @Synchronized
    fun release(): Boolean {
        if (activeLease == null) return false
        activeLease = null
        mutableState.value = DecoderSurfaceState(generation = generation)
        updateSurface(null)
        return true
    }
}
