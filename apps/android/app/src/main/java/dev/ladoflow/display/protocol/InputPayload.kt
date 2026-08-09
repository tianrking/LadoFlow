package dev.ladoflow.display.protocol

const val MAX_TOUCH_CONTACTS: Int = 16

private const val INPUT_HEADER_BYTES = 9

enum class ButtonState(val wireValue: Int) {
    Released(0),
    Pressed(1),
    ;

    companion object {
        fun fromWire(value: Int): ButtonState = entries.firstOrNull { it.wireValue == value }
            ?: protocolFailure(ProtocolViolation.InvalidPayload, "Unknown button state $value")
    }
}

enum class PointerButton(val wireValue: Int) {
    Primary(1),
    Secondary(2),
    Middle(3),
    Back(4),
    Forward(5),
    ;

    companion object {
        fun fromWire(value: Int): PointerButton = entries.firstOrNull { it.wireValue == value }
            ?: protocolFailure(ProtocolViolation.InvalidPayload, "Unknown pointer button $value")
    }
}

enum class TouchPhase(val wireValue: Int) {
    Begin(1),
    Move(2),
    End(3),
    Cancel(4),
    ;

    companion object {
        fun fromWire(value: Int): TouchPhase = entries.firstOrNull { it.wireValue == value }
            ?: protocolFailure(ProtocolViolation.InvalidPayload, "Unknown touch phase $value")
    }
}

data class KeyModifiers private constructor(val bits: Int) {
    fun contains(other: KeyModifiers): Boolean = bits and other.bits == other.bits

    infix fun or(other: KeyModifiers): KeyModifiers = fromBits(bits or other.bits)

    companion object {
        val None = KeyModifiers(0)
        val Shift = KeyModifiers(1 shl 0)
        val Control = KeyModifiers(1 shl 1)
        val Alt = KeyModifiers(1 shl 2)
        val Meta = KeyModifiers(1 shl 3)
        val CapsLock = KeyModifiers(1 shl 4)
        val NumLock = KeyModifiers(1 shl 5)

        private const val KNOWN_MASK =
            (1 shl 0) or (1 shl 1) or (1 shl 2) or (1 shl 3) or (1 shl 4) or (1 shl 5)

        fun fromBits(bits: Int): KeyModifiers {
            requireProtocol(bits and KNOWN_MASK.inv() == 0) {
                "Unknown keyboard modifier bits 0x${bits.toString(16)}"
            }
            return KeyModifiers(bits)
        }
    }
}

sealed interface InputEventBody {
    val wireKind: Int
    val encodedBytes: Int
}

data class PointerMoveInput(
    val x: Int,
    val y: Int,
) : InputEventBody {
    override val wireKind: Int = 1
    override val encodedBytes: Int = 13

    init {
        requireCoordinate(x, "x")
        requireCoordinate(y, "y")
    }
}

data class PointerButtonInput(
    val button: PointerButton,
    val state: ButtonState,
) : InputEventBody {
    override val wireKind: Int = 2
    override val encodedBytes: Int = 11
}

data class WheelInput(
    val deltaX: Short,
    val deltaY: Short,
) : InputEventBody {
    override val wireKind: Int = 3
    override val encodedBytes: Int = 13
}

data class KeyInput(
    val usage: Int,
    val state: ButtonState,
    val modifiers: KeyModifiers,
) : InputEventBody {
    override val wireKind: Int = 4
    override val encodedBytes: Int = 14

    init {
        requireProtocol(usage in 1..0xffff) { "Keyboard HID usage must be a non-zero u16" }
    }
}

data class TouchInput(
    val contactId: Int,
    val phase: TouchPhase,
    val x: Int,
    val y: Int,
    val pressure: Int,
) : InputEventBody {
    override val wireKind: Int = 5
    override val encodedBytes: Int = 17

    init {
        requireProtocol(contactId in 0 until MAX_TOUCH_CONTACTS) {
            "Touch contact identifier is out of range"
        }
        requireCoordinate(x, "x")
        requireCoordinate(y, "y")
        requireCoordinate(pressure, "pressure")
    }
}

data class FocusInput(val focused: Boolean) : InputEventBody {
    override val wireKind: Int = 6
    override val encodedBytes: Int = 10
}

data class InputPayload(
    val timestampMicros: ULong,
    val event: InputEventBody,
) : LdflPayload {
    override val messageType: MessageType = MessageType.Input

    override fun encode(): ByteArray {
        val writer = NetworkByteWriter(event.encodedBytes)
        writer.u64(timestampMicros)
        writer.u8(event.wireKind)
        when (event) {
            is PointerMoveInput -> {
                writer.u16(event.x)
                writer.u16(event.y)
            }

            is PointerButtonInput -> {
                writer.u8(event.button.wireValue)
                writer.u8(event.state.wireValue)
            }

            is WheelInput -> {
                writer.i16(event.deltaX)
                writer.i16(event.deltaY)
            }

            is KeyInput -> {
                writer.u16(event.usage)
                writer.u8(event.state.wireValue)
                writer.u16(event.modifiers.bits)
            }

            is TouchInput -> {
                writer.u8(event.contactId)
                writer.u8(event.phase.wireValue)
                writer.u16(event.x)
                writer.u16(event.y)
                writer.u16(event.pressure)
            }

            is FocusInput -> writer.u8(if (event.focused) 1 else 0)
        }
        return writer.toByteArray()
    }

    companion object {
        fun decode(payload: ByteArray): InputPayload {
            requireProtocol(payload.size >= INPUT_HEADER_BYTES) { "Input payload is truncated" }
            val event = when (val kind = payload.readU8(8)) {
                1 -> {
                    requireExactLength(payload, 13)
                    PointerMoveInput(payload.readU16(9), payload.readU16(11))
                }

                2 -> {
                    requireExactLength(payload, 11)
                    PointerButtonInput(
                        button = PointerButton.fromWire(payload.readU8(9)),
                        state = ButtonState.fromWire(payload.readU8(10)),
                    )
                }

                3 -> {
                    requireExactLength(payload, 13)
                    WheelInput(payload.readI16(9), payload.readI16(11))
                }

                4 -> {
                    requireExactLength(payload, 14)
                    KeyInput(
                        usage = payload.readU16(9),
                        state = ButtonState.fromWire(payload.readU8(11)),
                        modifiers = KeyModifiers.fromBits(payload.readU16(12)),
                    )
                }

                5 -> {
                    requireExactLength(payload, 17)
                    TouchInput(
                        contactId = payload.readU8(9),
                        phase = TouchPhase.fromWire(payload.readU8(10)),
                        x = payload.readU16(11),
                        y = payload.readU16(13),
                        pressure = payload.readU16(15),
                    )
                }

                6 -> {
                    requireExactLength(payload, 10)
                    FocusInput(decodeBoolean(payload.readU8(9)))
                }

                else -> protocolFailure(
                    ProtocolViolation.InvalidPayload,
                    "Unknown input event kind $kind",
                )
            }
            return InputPayload(payload.readU64(0), event)
        }

        private fun requireExactLength(payload: ByteArray, expected: Int) {
            requireProtocol(payload.size == expected) {
                "Input event length ${payload.size} does not match its kind ($expected)"
            }
        }

        private fun decodeBoolean(value: Int): Boolean = when (value) {
            0 -> false
            1 -> true
            else -> protocolFailure(
                ProtocolViolation.InvalidPayload,
                "Boolean input field must be zero or one",
            )
        }
    }
}

private fun requireCoordinate(value: Int, name: String) {
    requireProtocol(value in 0..0xffff) { "$name must fit an unsigned 16-bit field" }
}
