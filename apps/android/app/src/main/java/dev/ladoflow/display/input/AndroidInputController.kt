package dev.ladoflow.display.input

import android.view.InputDevice
import android.view.KeyEvent
import android.view.MotionEvent
import dev.ladoflow.display.protocol.ButtonState
import dev.ladoflow.display.protocol.FocusInput
import dev.ladoflow.display.protocol.InputPayload
import dev.ladoflow.display.protocol.KeyInput
import dev.ladoflow.display.protocol.PointerButton
import dev.ladoflow.display.protocol.PointerButtonInput
import dev.ladoflow.display.protocol.PointerMoveInput
import dev.ladoflow.display.protocol.TouchInput
import dev.ladoflow.display.protocol.TouchPhase
import dev.ladoflow.display.protocol.WheelInput
import kotlin.math.roundToInt

enum class InputDelivery {
    Critical,
    Coalescible,
}

data class AndroidInputEmission(
    val payload: InputPayload,
    val delivery: InputDelivery,
)

/** Android event adapter for a focusable decoder SurfaceView. */
class AndroidInputController(
    private val emit: (AndroidInputEmission) -> Unit,
) {
    private val touchContacts = TouchContactTracker()
    private val lastTouchPixels = mutableMapOf<Int, RemotePixel>()

    @Volatile
    private var viewport: RemoteViewport? = null

    fun updateViewport(viewport: RemoteViewport?) {
        if (this.viewport != viewport) cancelActiveTouches()
        this.viewport = viewport
    }

    fun onTouchEvent(
        viewWidth: Int,
        viewHeight: Int,
        event: MotionEvent,
    ): Boolean {
        if (event.isFromSource(InputDevice.SOURCE_MOUSE)) return false
        val transform = transform(viewWidth, viewHeight) ?: return false
        val timestamp = event.timestampMicros()
        return when (event.actionMasked) {
            MotionEvent.ACTION_DOWN,
            MotionEvent.ACTION_POINTER_DOWN,
            -> {
                val index = event.actionIndex
                val point = transform.map(event.getX(index), event.getY(index)) ?: return false
                val platformId = event.getPointerId(index)
                val contactId = touchContacts.begin(platformId) ?: return false
                lastTouchPixels[platformId] = point
                emit.touch(timestamp, contactId, TouchPhase.Begin, point, event.getPressure(index), true)
                true
            }

            MotionEvent.ACTION_MOVE -> {
                for (index in 0 until event.pointerCount) {
                    val platformId = event.getPointerId(index)
                    val contactId = touchContacts.contactId(platformId) ?: continue
                    val point = transform.mapClamped(event.getX(index), event.getY(index))
                    lastTouchPixels[platformId] = point
                    emit.touch(timestamp, contactId, TouchPhase.Move, point, event.getPressure(index), false)
                }
                true
            }

            MotionEvent.ACTION_UP,
            MotionEvent.ACTION_POINTER_UP,
            -> {
                val index = event.actionIndex
                val platformId = event.getPointerId(index)
                val contactId = touchContacts.contactId(platformId) ?: return false
                val point = transform.mapClamped(event.getX(index), event.getY(index))
                emit.touch(timestamp, contactId, TouchPhase.End, point, event.getPressure(index), true)
                touchContacts.end(platformId)
                lastTouchPixels.remove(platformId)
                true
            }

            MotionEvent.ACTION_CANCEL -> {
                touchContacts.cancelAll().forEach { (platformId, contactId) ->
                    val point = lastTouchPixels[platformId] ?: RemotePixel(0, 0)
                    emit.touch(timestamp, contactId, TouchPhase.Cancel, point, 0f, true)
                }
                lastTouchPixels.clear()
                true
            }

            else -> false
        }
    }

    fun onGenericMotionEvent(
        viewWidth: Int,
        viewHeight: Int,
        event: MotionEvent,
    ): Boolean {
        if (!event.isFromSource(InputDevice.SOURCE_MOUSE)) return false
        val transform = transform(viewWidth, viewHeight) ?: return false
        val point = transform.mapClamped(event.x, event.y)
        val timestamp = event.timestampMicros()
        return when (event.actionMasked) {
            MotionEvent.ACTION_HOVER_MOVE,
            MotionEvent.ACTION_MOVE,
            -> {
                emit(
                    AndroidInputEmission(
                        InputPayload(timestamp, PointerMoveInput(point.x, point.y)),
                        InputDelivery.Coalescible,
                    ),
                )
                true
            }

            MotionEvent.ACTION_BUTTON_PRESS,
            MotionEvent.ACTION_BUTTON_RELEASE,
            -> {
                val button = event.actionButton.toPointerButton() ?: return false
                emit(
                    AndroidInputEmission(
                        InputPayload(timestamp, PointerMoveInput(point.x, point.y)),
                        InputDelivery.Coalescible,
                    ),
                )
                emit(
                    AndroidInputEmission(
                        InputPayload(
                            timestamp,
                            PointerButtonInput(
                                button = button,
                                state = if (event.actionMasked == MotionEvent.ACTION_BUTTON_PRESS) {
                                    ButtonState.Pressed
                                } else {
                                    ButtonState.Released
                                },
                            ),
                        ),
                        InputDelivery.Critical,
                    ),
                )
                true
            }

            MotionEvent.ACTION_SCROLL -> {
                emit(
                    AndroidInputEmission(
                        InputPayload(
                            timestamp,
                            WheelInput(
                                deltaX = event.getAxisValue(MotionEvent.AXIS_HSCROLL).wheelDelta(),
                                deltaY = event.getAxisValue(MotionEvent.AXIS_VSCROLL).wheelDelta(),
                            ),
                        ),
                        InputDelivery.Coalescible,
                    ),
                )
                true
            }

            else -> false
        }
    }

    fun onKeyEvent(event: KeyEvent): Boolean {
        if (event.action != KeyEvent.ACTION_DOWN && event.action != KeyEvent.ACTION_UP) return false
        val usage = AndroidHidUsageMapper.usageForKeyCode(event.keyCode) ?: return false
        if (event.action == KeyEvent.ACTION_DOWN && event.repeatCount > 0) return true
        emit(
            AndroidInputEmission(
                payload = InputPayload(
                    timestampMicros = event.eventTime.coerceAtLeast(0).toULong() * MICROS_PER_MILLISECOND,
                    event = KeyInput(
                        usage = usage,
                        state = if (event.action == KeyEvent.ACTION_DOWN) {
                            ButtonState.Pressed
                        } else {
                            ButtonState.Released
                        },
                        modifiers = AndroidHidUsageMapper.modifiers(event.metaState),
                    ),
                ),
                delivery = InputDelivery.Critical,
            ),
        )
        return true
    }

    fun onFocusChanged(focused: Boolean) {
        emit(
            AndroidInputEmission(
                payload = InputPayload(
                    timestampMicros = android.os.SystemClock.elapsedRealtimeNanos().toULong() /
                        NANOS_PER_MICROSECOND,
                    event = FocusInput(focused),
                ),
                delivery = InputDelivery.Critical,
            ),
        )
    }

    private fun cancelActiveTouches() {
        val timestamp = android.os.SystemClock.elapsedRealtimeNanos().toULong() /
            NANOS_PER_MICROSECOND
        touchContacts.cancelAll().forEach { (platformId, contactId) ->
            emit.touch(
                timestamp = timestamp,
                contactId = contactId,
                phase = TouchPhase.Cancel,
                point = lastTouchPixels[platformId] ?: RemotePixel(0, 0),
                pressure = 0f,
                critical = true,
            )
        }
        lastTouchPixels.clear()
    }

    private fun transform(
        viewWidth: Int,
        viewHeight: Int,
    ): ViewportTransform? {
        val currentViewport = viewport ?: return null
        if (viewWidth <= 0 || viewHeight <= 0) return null
        return ViewportTransform(viewWidth, viewHeight, currentViewport)
    }

    private fun ((AndroidInputEmission) -> Unit).touch(
        timestamp: ULong,
        contactId: Int,
        phase: TouchPhase,
        point: RemotePixel,
        pressure: Float,
        critical: Boolean,
    ) {
        invoke(
            AndroidInputEmission(
                payload = InputPayload(
                    timestamp,
                    TouchInput(
                        contactId = contactId,
                        phase = phase,
                        x = point.x,
                        y = point.y,
                        pressure = pressure.toU16Pressure(),
                    ),
                ),
                delivery = if (critical) InputDelivery.Critical else InputDelivery.Coalescible,
            ),
        )
    }

    private fun MotionEvent.timestampMicros(): ULong =
        eventTime.coerceAtLeast(0).toULong() * MICROS_PER_MILLISECOND

    private fun Float.wheelDelta(): Short =
        ((takeIf(Float::isFinite) ?: 0f) * WHEEL_UNITS_PER_AXIS).roundToInt()
            .coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt())
            .toShort()

    private fun Float.toU16Pressure(): Int =
        ((takeIf(Float::isFinite) ?: 0f).coerceIn(0f, 1f) * U16_MAX).roundToInt()

    private fun Int.toPointerButton(): PointerButton? = when (this) {
        MotionEvent.BUTTON_PRIMARY -> PointerButton.Primary
        MotionEvent.BUTTON_SECONDARY -> PointerButton.Secondary
        MotionEvent.BUTTON_TERTIARY -> PointerButton.Middle
        MotionEvent.BUTTON_BACK -> PointerButton.Back
        MotionEvent.BUTTON_FORWARD -> PointerButton.Forward
        else -> null
    }

    companion object {
        private const val U16_MAX = 65_535
        private const val WHEEL_UNITS_PER_AXIS = 120f
        private const val MICROS_PER_MILLISECOND = 1_000uL
        private const val NANOS_PER_MICROSECOND = 1_000uL
    }
}
