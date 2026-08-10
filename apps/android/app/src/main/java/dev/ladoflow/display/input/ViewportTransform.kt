package dev.ladoflow.display.input

import kotlin.math.min
import kotlin.math.roundToInt

enum class DisplayRotation {
    Degrees0,
    Degrees90,
    Degrees180,
    Degrees270,
}

data class RemoteViewport(
    val codedWidth: Int,
    val codedHeight: Int,
    val rotation: DisplayRotation = DisplayRotation.Degrees0,
) {
    init {
        require(codedWidth in 1..0xffff)
        require(codedHeight in 1..0xffff)
    }
}

data class RemotePixel(
    val x: Int,
    val y: Int,
)

/** Maps a fit-center Android view coordinate into LDFL's coded-pixel space. */
class ViewportTransform(
    private val viewWidth: Int,
    private val viewHeight: Int,
    private val viewport: RemoteViewport,
) {
    private val rotatedWidth = when (viewport.rotation) {
        DisplayRotation.Degrees0,
        DisplayRotation.Degrees180,
        -> viewport.codedWidth

        DisplayRotation.Degrees90,
        DisplayRotation.Degrees270,
        -> viewport.codedHeight
    }
    private val rotatedHeight = when (viewport.rotation) {
        DisplayRotation.Degrees0,
        DisplayRotation.Degrees180,
        -> viewport.codedHeight

        DisplayRotation.Degrees90,
        DisplayRotation.Degrees270,
        -> viewport.codedWidth
    }
    private val scale = min(
        viewWidth.toDouble() / rotatedWidth.toDouble(),
        viewHeight.toDouble() / rotatedHeight.toDouble(),
    )
    private val contentWidth = rotatedWidth * scale
    private val contentHeight = rotatedHeight * scale
    private val contentLeft = (viewWidth - contentWidth) / 2.0
    private val contentTop = (viewHeight - contentHeight) / 2.0

    init {
        require(viewWidth > 0)
        require(viewHeight > 0)
    }

    fun map(
        viewX: Float,
        viewY: Float,
    ): RemotePixel? {
        if (
            viewX < contentLeft ||
            viewX > contentLeft + contentWidth ||
            viewY < contentTop ||
            viewY > contentTop + contentHeight
        ) {
            return null
        }
        return mapNormalized(
            displayedX = ((viewX - contentLeft) / contentWidth).coerceIn(0.0, 1.0),
            displayedY = ((viewY - contentTop) / contentHeight).coerceIn(0.0, 1.0),
        )
    }

    fun mapClamped(
        viewX: Float,
        viewY: Float,
    ): RemotePixel = mapNormalized(
        displayedX = ((viewX - contentLeft) / contentWidth).coerceIn(0.0, 1.0),
        displayedY = ((viewY - contentTop) / contentHeight).coerceIn(0.0, 1.0),
    )

    private fun mapNormalized(
        displayedX: Double,
        displayedY: Double,
    ): RemotePixel {
        val sourceX: Double
        val sourceY: Double
        when (viewport.rotation) {
            DisplayRotation.Degrees0 -> {
                sourceX = displayedX
                sourceY = displayedY
            }

            DisplayRotation.Degrees90 -> {
                sourceX = displayedY
                sourceY = 1.0 - displayedX
            }

            DisplayRotation.Degrees180 -> {
                sourceX = 1.0 - displayedX
                sourceY = 1.0 - displayedY
            }

            DisplayRotation.Degrees270 -> {
                sourceX = 1.0 - displayedY
                sourceY = displayedX
            }
        }
        return RemotePixel(
            x = (sourceX * (viewport.codedWidth - 1)).roundToInt()
                .coerceIn(0, viewport.codedWidth - 1),
            y = (sourceY * (viewport.codedHeight - 1)).roundToInt()
                .coerceIn(0, viewport.codedHeight - 1),
        )
    }
}
