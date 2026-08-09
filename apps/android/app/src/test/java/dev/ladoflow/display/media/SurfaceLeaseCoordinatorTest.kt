package dev.ladoflow.display.media

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SurfaceLeaseCoordinatorTest {
    @Test
    fun staleActivityDetachCannotClearNewSurface() {
        val surfaceUpdates = mutableListOf<String?>()
        val coordinator = SurfaceLeaseCoordinator<String>(surfaceUpdates::add)
        val oldActivity = coordinator.attach("portrait surface")
        val newActivity = coordinator.attach("landscape surface")

        assertFalse(coordinator.detach(oldActivity))
        assertEquals(listOf("portrait surface", "landscape surface"), surfaceUpdates)
        assertEquals(
            DecoderSurfaceState(generation = 2, attached = true),
            coordinator.state.value,
        )

        assertTrue(coordinator.detach(newActivity))
        assertEquals(null, surfaceUpdates.last())
        assertEquals(DecoderSurfaceState(generation = 2), coordinator.state.value)
    }

    @Test
    fun directionAndSizeChangesStayOnCurrentSurfaceLease() {
        val surfaceUpdates = mutableListOf<String?>()
        val coordinator = SurfaceLeaseCoordinator<String>(surfaceUpdates::add)
        val lease = coordinator.attach("surface")

        assertTrue(coordinator.resize(lease, width = 1_080, height = 1_920))
        assertEquals(
            DecoderSurfaceState(
                generation = 1,
                attached = true,
                width = 1_080,
                height = 1_920,
            ),
            coordinator.state.value,
        )
        assertTrue(coordinator.resize(lease, width = 1_920, height = 1_080))
        assertEquals(1_920, coordinator.state.value.width)
        assertEquals(1_080, coordinator.state.value.height)
        assertEquals(listOf("surface"), surfaceUpdates)
    }

    @Test
    fun staleOrInvalidResizeIsIgnored() {
        val coordinator = SurfaceLeaseCoordinator<String> { }
        val staleLease = coordinator.attach("old")
        val activeLease = coordinator.attach("new")

        assertFalse(coordinator.resize(staleLease, 1_920, 1_080))
        assertFalse(coordinator.resize(activeLease, 0, 1_080))
        assertEquals(DecoderSurfaceState(generation = 2, attached = true), coordinator.state.value)
    }
}
