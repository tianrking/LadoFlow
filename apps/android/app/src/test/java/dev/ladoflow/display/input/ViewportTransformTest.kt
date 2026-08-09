package dev.ladoflow.display.input

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ViewportTransformTest {
    @Test
    fun fitCenterRejectsLetterboxAndMapsCodedEdges() {
        val transform = ViewportTransform(
            viewWidth = 1_000,
            viewHeight = 1_000,
            viewport = RemoteViewport(1_920, 1_080),
        )

        assertNull(transform.map(500f, 100f))
        assertEquals(RemotePixel(0, 0), transform.map(0f, 218.75f))
        assertEquals(RemotePixel(1_919, 1_079), transform.map(1_000f, 781.25f))
        assertEquals(RemotePixel(960, 540), transform.map(500f, 500f))
        assertEquals(RemotePixel(960, 0), transform.mapClamped(500f, -100f))
    }

    @Test
    fun clockwiseQuarterTurnMapsDisplayedCornersBackToCodedPixels() {
        val transform = ViewportTransform(
            viewWidth = 300,
            viewHeight = 400,
            viewport = RemoteViewport(4, 3, DisplayRotation.Degrees90),
        )

        assertEquals(RemotePixel(0, 2), transform.map(0f, 0f))
        assertEquals(RemotePixel(0, 0), transform.map(300f, 0f))
        assertEquals(RemotePixel(3, 2), transform.map(0f, 400f))
        assertEquals(RemotePixel(3, 0), transform.map(300f, 400f))
    }

    @Test
    fun halfAndCounterClockwiseTurnsMapCorners() {
        val halfTurn = ViewportTransform(
            400,
            300,
            RemoteViewport(4, 3, DisplayRotation.Degrees180),
        )
        assertEquals(RemotePixel(3, 2), halfTurn.map(0f, 0f))
        assertEquals(RemotePixel(0, 0), halfTurn.map(400f, 300f))

        val counterClockwise = ViewportTransform(
            300,
            400,
            RemoteViewport(4, 3, DisplayRotation.Degrees270),
        )
        assertEquals(RemotePixel(3, 0), counterClockwise.map(0f, 0f))
        assertEquals(RemotePixel(0, 2), counterClockwise.map(300f, 400f))
    }

    @Test
    fun newDisplayConfigDimensionsCreateANewPixelMapping() {
        val landscape = ViewportTransform(1_920, 1_080, RemoteViewport(1_920, 1_080))
        val portrait = ViewportTransform(1_080, 1_920, RemoteViewport(1_080, 1_920))

        assertEquals(RemotePixel(1_919, 1_079), landscape.map(1_920f, 1_080f))
        assertEquals(RemotePixel(1_079, 1_919), portrait.map(1_080f, 1_920f))
    }
}
