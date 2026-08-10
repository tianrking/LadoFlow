package dev.ladoflow.display.protocol

import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets

internal class NetworkByteWriter(initialCapacity: Int) {
    private val output = ByteArrayOutputStream(initialCapacity)

    fun u8(value: Int) {
        requireProtocol(value in 0..0xff) { "u8 value is out of range" }
        output.write(value)
    }

    fun u16(value: Int) {
        requireProtocol(value in 0..0xffff) { "u16 value is out of range" }
        output.write(value ushr 8)
        output.write(value)
    }

    fun i16(value: Short) {
        u16(value.toInt() and 0xffff)
    }

    fun u32(value: UInt) {
        output.write((value shr 24).toInt())
        output.write((value shr 16).toInt())
        output.write((value shr 8).toInt())
        output.write(value.toInt())
    }

    fun u64(value: ULong) {
        for (shift in 56 downTo 0 step 8) {
            output.write((value shr shift).toInt())
        }
    }

    fun bytes(value: ByteArray) {
        output.write(value)
    }

    fun toByteArray(): ByteArray = output.toByteArray()
}

internal fun ByteArray.readU8(offset: Int): Int = this[offset].toInt() and 0xff

internal fun ByteArray.readU16(offset: Int): Int =
    (readU8(offset) shl 8) or readU8(offset + 1)

internal fun ByteArray.readI16(offset: Int): Short = readU16(offset).toShort()

internal fun ByteArray.readU32(offset: Int): UInt =
    ((readU8(offset).toUInt() shl 24) or
        (readU8(offset + 1).toUInt() shl 16) or
        (readU8(offset + 2).toUInt() shl 8) or
        readU8(offset + 3).toUInt())

internal fun ByteArray.readU64(offset: Int): ULong {
    var value = 0uL
    repeat(8) { index ->
        value = (value shl 8) or readU8(offset + index).toULong()
    }
    return value
}

internal fun ByteArray.decodeStrictUtf8(): String {
    val decoder = StandardCharsets.UTF_8.newDecoder()
        .onMalformedInput(CodingErrorAction.REPORT)
        .onUnmappableCharacter(CodingErrorAction.REPORT)
    return try {
        decoder.decode(ByteBuffer.wrap(this)).toString()
    } catch (_: Exception) {
        protocolFailure(ProtocolViolation.InvalidUtf8, "Protocol string is not valid UTF-8")
    }
}
