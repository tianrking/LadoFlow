package dev.ladoflow.display.protocol

const val MAX_STAGE_DURATION_MICROS: UInt = 60_000_000u
const val MAX_TELEMETRY_QUEUE_DEPTH: Int = 4_096
const val MAX_LOSS_PARTS_PER_MILLION: UInt = 1_000_000u

private const val TELEMETRY_BYTES = 51

enum class ThermalState(val wireValue: Int) {
    Unknown(0),
    Nominal(1),
    Fair(2),
    Serious(3),
    Critical(4),
    ;

    companion object {
        fun fromWire(value: Int): ThermalState = entries.firstOrNull { it.wireValue == value }
            ?: protocolFailure(ProtocolViolation.InvalidPayload, "Unknown thermal state $value")
    }
}

data class StageTimings(
    val captureMicros: UInt,
    val encodeMicros: UInt,
    val transportMicros: UInt,
    val decodeMicros: UInt,
    val presentationMicros: UInt,
) {
    init {
        requireProtocol(
            listOf(
                captureMicros,
                encodeMicros,
                transportMicros,
                decodeMicros,
                presentationMicros,
            ).all { it <= MAX_STAGE_DURATION_MICROS },
        ) { "Telemetry stage duration exceeds the version-one limit" }
    }

    internal fun encodeInto(writer: NetworkByteWriter) {
        writer.u32(captureMicros)
        writer.u32(encodeMicros)
        writer.u32(transportMicros)
        writer.u32(decodeMicros)
        writer.u32(presentationMicros)
    }
}

data class TelemetryPayload(
    val sampleTimestampMicros: ULong,
    val frameId: ULong,
    val timings: StageTimings,
    val queueDepth: Int,
    val lossPartsPerMillion: UInt,
    val droppedFrames: UInt,
    val lateFrames: UInt,
    val thermalState: ThermalState,
) : LdflPayload {
    override val messageType: MessageType = MessageType.Telemetry

    init {
        requireProtocol(queueDepth in 0..MAX_TELEMETRY_QUEUE_DEPTH) {
            "Telemetry queue depth exceeds the version-one limit"
        }
        requireProtocol(lossPartsPerMillion <= MAX_LOSS_PARTS_PER_MILLION) {
            "Telemetry loss exceeds one million parts per million"
        }
    }

    override fun encode(): ByteArray {
        val writer = NetworkByteWriter(TELEMETRY_BYTES)
        writer.u64(sampleTimestampMicros)
        writer.u64(frameId)
        timings.encodeInto(writer)
        writer.u16(queueDepth)
        writer.u32(lossPartsPerMillion)
        writer.u32(droppedFrames)
        writer.u32(lateFrames)
        writer.u8(thermalState.wireValue)
        return writer.toByteArray()
    }

    companion object {
        fun decode(payload: ByteArray): TelemetryPayload {
            requireProtocol(payload.size == TELEMETRY_BYTES) {
                "Telemetry payload must be exactly $TELEMETRY_BYTES bytes"
            }
            return TelemetryPayload(
                sampleTimestampMicros = payload.readU64(0),
                frameId = payload.readU64(8),
                timings = StageTimings(
                    captureMicros = payload.readU32(16),
                    encodeMicros = payload.readU32(20),
                    transportMicros = payload.readU32(24),
                    decodeMicros = payload.readU32(28),
                    presentationMicros = payload.readU32(32),
                ),
                queueDepth = payload.readU16(36),
                lossPartsPerMillion = payload.readU32(38),
                droppedFrames = payload.readU32(42),
                lateFrames = payload.readU32(46),
                thermalState = ThermalState.fromWire(payload.readU8(50)),
            )
        }
    }
}
