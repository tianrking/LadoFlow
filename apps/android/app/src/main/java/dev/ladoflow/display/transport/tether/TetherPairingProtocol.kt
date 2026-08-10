package dev.ladoflow.display.transport.tether

import java.io.EOFException
import java.io.InputStream
import java.io.OutputStream
import java.nio.charset.StandardCharsets
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

internal const val TETHER_PAIRING_RECORD_BYTES: Int = 56
internal const val TETHER_PAIRING_TOKEN_BYTES: Int = 10
internal const val TETHER_PAIRING_NONCE_BYTES: Int = 16
internal const val TETHER_PAIRING_TAG_BYTES: Int = 32

internal enum class TetherPairingKind(val wireValue: Int) {
    HostHello(1),
    DisplayHello(2),
    HostFinished(3),
    DisplayFinished(4),
    ;

    companion object {
        fun fromWire(value: Int): TetherPairingKind = entries.firstOrNull {
            it.wireValue == value
        } ?: throw TetherPairingException("Unknown USB tether pairing record kind $value")
    }
}

/** Redacts itself so a state or diagnostic object cannot accidentally log the pairing code. */
class TetherPairingCode internal constructor(
    private val rendered: String,
) {
    fun revealForDisplay(): String = rendered

    override fun equals(other: Any?): Boolean =
        other is TetherPairingCode && rendered == other.rendered

    override fun hashCode(): Int = rendered.hashCode()

    override fun toString(): String = "[redacted USB tether pairing code]"
}

/** Owns the memory-only raw key and zeroes it when the listener stops or authenticates. */
internal class TetherPairingToken private constructor(
    private val key: ByteArray,
) {
    val displayCode: TetherPairingCode = TetherPairingCode(formatCrockfordBase32(key))

    private var valid = true

    @Synchronized
    fun computeTag(
        kind: TetherPairingKind,
        hostNonce: ByteArray,
        displayNonce: ByteArray,
    ): ByteArray {
        check(valid) { "USB tether pairing token is no longer valid" }
        val keyCopy = key.copyOf()
        return try {
            computePairingTag(keyCopy, kind, hostNonce, displayNonce)
        } finally {
            keyCopy.fill(0)
        }
    }

    @Synchronized
    fun invalidate() {
        if (!valid) return
        valid = false
        key.fill(0)
    }

    @Synchronized
    fun isValid(): Boolean = valid

    override fun toString(): String = "[redacted USB tether pairing token]"

    companion object {
        fun generate(random: SecureRandom = SecureRandom()): TetherPairingToken =
            TetherPairingToken(ByteArray(TETHER_PAIRING_TOKEN_BYTES).also(random::nextBytes))

        fun fromBytes(bytes: ByteArray): TetherPairingToken {
            require(bytes.size == TETHER_PAIRING_TOKEN_BYTES) {
                "USB tether pairing token must contain $TETHER_PAIRING_TOKEN_BYTES bytes"
            }
            return TetherPairingToken(bytes.copyOf())
        }
    }
}

internal data class TetherPairingRecord(
    val kind: TetherPairingKind,
    val nonce: ByteArray,
    val tag: ByteArray,
) {
    override fun equals(other: Any?): Boolean = other is TetherPairingRecord &&
        kind == other.kind &&
        nonce.contentEquals(other.nonce) &&
        tag.contentEquals(other.tag)

    override fun hashCode(): Int = 31 * (31 * kind.hashCode() + nonce.contentHashCode()) +
        tag.contentHashCode()
}

internal data class TetherPairingHandshake(
    val hostNonce: ByteArray,
    val displayNonce: ByteArray,
)

internal class TetherPairingException(message: String) : Exception(message)

/**
 * Executes exactly two fixed-size reads. It deliberately avoids buffered streams so the first
 * post-pairing byte remains untouched for the raw LDFL incremental decoder.
 */
internal fun performDisplayPairingHandshake(
    input: InputStream,
    output: OutputStream,
    token: TetherPairingToken,
    displayNonceSource: () -> ByteArray = ::securePairingNonce,
): TetherPairingHandshake {
    val hostHello = readPairingRecord(input, TetherPairingKind.HostHello)
    val hostNonce = hostHello.nonce
    val displayNonce = displayNonceSource().copyOf()
    if (displayNonce.size != TETHER_PAIRING_NONCE_BYTES || displayNonce.all { it == 0.toByte() }) {
        throw TetherPairingException(
            "Display nonce must contain $TETHER_PAIRING_NONCE_BYTES bytes and must not be zero",
        )
    }

    val displayHelloTag = token.computeTag(
        TetherPairingKind.DisplayHello,
        hostNonce,
        displayNonce,
    )
    writePairingRecord(
        output,
        TetherPairingRecord(
            kind = TetherPairingKind.DisplayHello,
            nonce = displayNonce,
            tag = displayHelloTag,
        ),
    )

    val hostFinished = readPairingRecord(input, TetherPairingKind.HostFinished)
    val expectedHostTag = token.computeTag(
        TetherPairingKind.HostFinished,
        hostNonce,
        displayNonce,
    )
    val authenticated = MessageDigest.isEqual(expectedHostTag, hostFinished.tag)
    expectedHostTag.fill(0)
    if (!authenticated) {
        throw TetherPairingException("HostFinished authentication tag did not match")
    }

    val displayFinishedTag = token.computeTag(
        TetherPairingKind.DisplayFinished,
        hostNonce,
        displayNonce,
    )
    writePairingRecord(
        output,
        TetherPairingRecord(
            kind = TetherPairingKind.DisplayFinished,
            nonce = ByteArray(TETHER_PAIRING_NONCE_BYTES),
            tag = displayFinishedTag,
        ),
    )
    return TetherPairingHandshake(hostNonce.copyOf(), displayNonce.copyOf())
}

internal fun encodePairingRecord(record: TetherPairingRecord): ByteArray {
    validateCanonicalRecord(record)
    return ByteArray(TETHER_PAIRING_RECORD_BYTES).also { encoded ->
        TETHER_PAIRING_MAGIC.copyInto(encoded, 0)
        encoded[4] = 0
        encoded[5] = TETHER_PAIRING_VERSION.toByte()
        encoded[6] = record.kind.wireValue.toByte()
        encoded[7] = 0
        record.nonce.copyInto(encoded, 8)
        record.tag.copyInto(encoded, 8 + TETHER_PAIRING_NONCE_BYTES)
    }
}

internal fun readPairingRecord(
    input: InputStream,
    expectedKind: TetherPairingKind,
): TetherPairingRecord {
    val encoded = readExactly(input, TETHER_PAIRING_RECORD_BYTES)
    if (!encoded.copyOfRange(0, 4).contentEquals(TETHER_PAIRING_MAGIC)) {
        throw TetherPairingException("USB tether pairing magic must be LDFP")
    }
    val version = ((encoded[4].toInt() and 0xff) shl 8) or (encoded[5].toInt() and 0xff)
    if (version != TETHER_PAIRING_VERSION) {
        throw TetherPairingException("USB tether pairing version must be 1")
    }
    val kind = TetherPairingKind.fromWire(encoded[6].toInt() and 0xff)
    if (kind != expectedKind) {
        throw TetherPairingException(
            "Expected ${expectedKind.name} but received ${kind.name}",
        )
    }
    if (encoded[7] != 0.toByte()) {
        throw TetherPairingException("USB tether pairing reserved byte must be zero")
    }
    val record = TetherPairingRecord(
        kind = kind,
        nonce = encoded.copyOfRange(8, 8 + TETHER_PAIRING_NONCE_BYTES),
        tag = encoded.copyOfRange(8 + TETHER_PAIRING_NONCE_BYTES, encoded.size),
    )
    validateCanonicalRecord(record)
    return record
}

internal fun computePairingTag(
    token: ByteArray,
    kind: TetherPairingKind,
    hostNonce: ByteArray,
    displayNonce: ByteArray,
): ByteArray {
    require(token.size == TETHER_PAIRING_TOKEN_BYTES)
    require(hostNonce.size == TETHER_PAIRING_NONCE_BYTES)
    require(displayNonce.size == TETHER_PAIRING_NONCE_BYTES)
    require(kind != TetherPairingKind.HostHello) {
        "HostHello carries a zero tag and is not authenticated"
    }
    val message = PAIRING_CONTEXT +
        byteArrayOf(0, 0, TETHER_PAIRING_VERSION.toByte(), kind.wireValue.toByte()) +
        hostNonce +
        displayNonce
    val mac = Mac.getInstance("HmacSHA256")
    mac.init(SecretKeySpec(token, "HmacSHA256"))
    return mac.doFinal(message)
}

private fun writePairingRecord(output: OutputStream, record: TetherPairingRecord) {
    output.write(encodePairingRecord(record))
    output.flush()
}

private fun validateCanonicalRecord(record: TetherPairingRecord) {
    if (record.nonce.size != TETHER_PAIRING_NONCE_BYTES) {
        throw TetherPairingException("USB tether pairing nonce must contain 16 bytes")
    }
    if (record.tag.size != TETHER_PAIRING_TAG_BYTES) {
        throw TetherPairingException("USB tether pairing tag must contain 32 bytes")
    }
    val nonceIsZero = record.nonce.all { it == 0.toByte() }
    val tagIsZero = record.tag.all { it == 0.toByte() }
    when (record.kind) {
        TetherPairingKind.HostHello -> {
            if (nonceIsZero) throw TetherPairingException("HostHello nonce must not be zero")
            if (!tagIsZero) throw TetherPairingException("HostHello tag must be zero")
        }

        TetherPairingKind.DisplayHello -> {
            if (nonceIsZero) throw TetherPairingException("DisplayHello nonce must not be zero")
            if (tagIsZero) throw TetherPairingException("DisplayHello tag must not be zero")
        }

        TetherPairingKind.HostFinished,
        TetherPairingKind.DisplayFinished,
        -> {
            if (!nonceIsZero) {
                throw TetherPairingException("${record.kind.name} nonce must be zero")
            }
            if (tagIsZero) throw TetherPairingException("${record.kind.name} tag must not be zero")
        }
    }
}

private fun readExactly(input: InputStream, byteCount: Int): ByteArray {
    val result = ByteArray(byteCount)
    var offset = 0
    while (offset < byteCount) {
        val read = input.read(result, offset, byteCount - offset)
        if (read < 0) {
            throw EOFException(
                "USB tether pairing stream ended after $offset of $byteCount bytes",
            )
        }
        if (read == 0) throw TetherPairingException("USB tether pairing read made no progress")
        offset += read
    }
    return result
}

private fun securePairingNonce(): ByteArray {
    val random = SecureRandom()
    repeat(8) {
        val nonce = ByteArray(TETHER_PAIRING_NONCE_BYTES).also(random::nextBytes)
        if (nonce.any { it != 0.toByte() }) return nonce
    }
    throw TetherPairingException("SecureRandom repeatedly returned a zero display nonce")
}

private fun formatCrockfordBase32(bytes: ByteArray): String {
    require(bytes.size == TETHER_PAIRING_TOKEN_BYTES)
    val compact = buildString(16) {
        repeat(16) { group ->
            var value = 0
            repeat(5) { bitInGroup ->
                val bitIndex = group * 5 + bitInGroup
                val byte = bytes[bitIndex / 8].toInt() and 0xff
                value = (value shl 1) or ((byte shr (7 - bitIndex % 8)) and 1)
            }
            append(CROCKFORD_ALPHABET[value])
        }
    }
    return compact.chunked(4).joinToString("-")
}

private const val TETHER_PAIRING_VERSION = 1
private val TETHER_PAIRING_MAGIC = "LDFP".toByteArray(StandardCharsets.US_ASCII)
private val PAIRING_CONTEXT =
    "LadoFlow USB tether pairing v1".toByteArray(StandardCharsets.US_ASCII)
private const val CROCKFORD_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
