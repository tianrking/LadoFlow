package dev.ladoflow.display.transport.tether

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.InputStream
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class TetherPairingProtocolTest {
    private val tokenBytes = "00010203040506070809".hexBytes()
    private val hostNonce = ByteArray(16) { 0x11 }
    private val displayNonce = ByteArray(16) { 0x22 }

    @Test
    fun `Windows golden HMAC vectors match exactly`() {
        assertEquals(
            "73d8fcaffc575ef3fc87af45db2f900e3d497b2defa946d034f676b6735d3ddc",
            computePairingTag(
                tokenBytes,
                TetherPairingKind.DisplayHello,
                hostNonce,
                displayNonce,
            ).hex(),
        )
        assertEquals(
            "33eaeed1a55812212c0ae49c5b57b1fede4e0fdcd533d80bc0d1acba3f9d1ef5",
            computePairingTag(
                tokenBytes,
                TetherPairingKind.HostFinished,
                hostNonce,
                displayNonce,
            ).hex(),
        )
        assertEquals(
            "6c110e9833d57f9a19cff9a21a3507dbb02cf6d97cfd69bfc1f043b6f08baded",
            computePairingTag(
                tokenBytes,
                TetherPairingKind.DisplayFinished,
                hostNonce,
                displayNonce,
            ).hex(),
        )
    }

    @Test
    fun `ten raw bytes render as sixteen grouped Crockford characters and redact logs`() {
        val token = TetherPairingToken.fromBytes(tokenBytes)

        assertEquals("000G-40R4-0M30-E209", token.displayCode.revealForDisplay())
        assertFalse(token.displayCode.toString().contains("000G"))
        assertFalse(token.toString().contains("000102"))
        assertFalse(
            UsbTetherPairingState.Listening(
                address = "192.168.42.129",
                port = 49_231,
                code = token.displayCode,
                expiresAfterSeconds = 120,
                failedHandshakes = 0,
                maximumFailedHandshakes = 3,
            ).toString().contains("000G"),
        )

        token.invalidate()
        assertFalse(token.isValid())
        assertThrows(IllegalStateException::class.java) {
            token.computeTag(TetherPairingKind.DisplayHello, hostNonce, displayNonce)
        }
    }

    @Test
    fun `display handshake uses four exact records and leaves first LDFL byte unread`() {
        val token = TetherPairingToken.fromBytes(tokenBytes)
        val hostHello = encodePairingRecord(
            TetherPairingRecord(
                TetherPairingKind.HostHello,
                hostNonce,
                ByteArray(32),
            ),
        )
        val hostFinished = encodePairingRecord(
            TetherPairingRecord(
                TetherPairingKind.HostFinished,
                ByteArray(16),
                computePairingTag(
                    tokenBytes,
                    TetherPairingKind.HostFinished,
                    hostNonce,
                    displayNonce,
                ),
            ),
        )
        val firstLdflBytes = byteArrayOf(0x4c, 0x44, 0x46, 0x4c)
        val input = ChunkLimitedInputStream(hostHello + hostFinished + firstLdflBytes, 3)
        val output = ByteArrayOutputStream()

        val handshake = performDisplayPairingHandshake(
            input,
            output,
            token,
            displayNonceSource = { displayNonce },
        )

        assertArrayEquals(hostNonce, handshake.hostNonce)
        assertArrayEquals(displayNonce, handshake.displayNonce)
        assertEquals(TETHER_PAIRING_RECORD_BYTES * 2, output.size())
        val displayRecords = ByteArrayInputStream(output.toByteArray())
        val displayHello = readPairingRecord(
            displayRecords,
            TetherPairingKind.DisplayHello,
        )
        val displayFinished = readPairingRecord(
            displayRecords,
            TetherPairingKind.DisplayFinished,
        )
        assertArrayEquals(displayNonce, displayHello.nonce)
        assertEquals(
            "73d8fcaffc575ef3fc87af45db2f900e3d497b2defa946d034f676b6735d3ddc",
            displayHello.tag.hex(),
        )
        assertEquals(
            "6c110e9833d57f9a19cff9a21a3507dbb02cf6d97cfd69bfc1f043b6f08baded",
            displayFinished.tag.hex(),
        )
        assertEquals(0, displayRecords.available())
        assertArrayEquals(firstLdflBytes, input.readBytes())
    }

    @Test
    fun `wrong HostFinished tag fails closed before DisplayFinished`() {
        val hostHello = encodePairingRecord(
            TetherPairingRecord(TetherPairingKind.HostHello, hostNonce, ByteArray(32)),
        )
        val hostFinished = encodePairingRecord(
            TetherPairingRecord(
                TetherPairingKind.HostFinished,
                ByteArray(16),
                ByteArray(32) { 0x55 },
            ),
        )
        val output = ByteArrayOutputStream()

        assertThrows(TetherPairingException::class.java) {
            performDisplayPairingHandshake(
                ByteArrayInputStream(hostHello + hostFinished),
                output,
                TetherPairingToken.fromBytes(tokenBytes),
                displayNonceSource = { displayNonce },
            )
        }

        assertEquals(TETHER_PAIRING_RECORD_BYTES, output.size())
        readPairingRecord(
            ByteArrayInputStream(output.toByteArray()),
            TetherPairingKind.DisplayHello,
        )
    }

    @Test
    fun `every noncanonical HostHello field is rejected`() {
        val canonical = encodePairingRecord(
            TetherPairingRecord(TetherPairingKind.HostHello, hostNonce, ByteArray(32)),
        )
        val corruptions = listOf(
            canonical.copyOf().also { it[0] = 0 },
            canonical.copyOf().also { it[4] = 1 },
            canonical.copyOf().also { it[5] = 2 },
            canonical.copyOf().also { it[6] = TetherPairingKind.DisplayHello.wireValue.toByte() },
            canonical.copyOf().also { it[7] = 1 },
            canonical.copyOf().also { it.fill(0, 8, 24) },
            canonical.copyOf().also { it[24] = 1 },
        )

        corruptions.forEach { bytes ->
            assertThrows(Exception::class.java) {
                readPairingRecord(
                    ByteArrayInputStream(bytes),
                    TetherPairingKind.HostHello,
                )
            }
        }
    }

    @Test
    fun `finished records reject nonzero nonce and zero tag`() {
        val validTag = ByteArray(32) { 0x33 }
        val nonzeroNonce = TetherPairingRecord(
            TetherPairingKind.HostFinished,
            ByteArray(16).also { it[15] = 1 },
            validTag,
        )
        val zeroTag = TetherPairingRecord(
            TetherPairingKind.HostFinished,
            ByteArray(16),
            ByteArray(32),
        )

        assertThrows(TetherPairingException::class.java) { encodePairingRecord(nonzeroNonce) }
        assertThrows(TetherPairingException::class.java) { encodePairingRecord(zeroTag) }
    }

    @Test
    fun `short record never crosses into an implicit next phase`() {
        val short = ByteArray(TETHER_PAIRING_RECORD_BYTES - 1)
        assertThrows(Exception::class.java) {
            readPairingRecord(ByteArrayInputStream(short), TetherPairingKind.HostHello)
        }
    }
}

private class ChunkLimitedInputStream(
    private val bytes: ByteArray,
    private val maximumChunk: Int,
) : InputStream() {
    private var offset = 0

    override fun read(): Int = if (offset == bytes.size) {
        -1
    } else {
        bytes[offset++].toInt() and 0xff
    }

    override fun read(target: ByteArray, targetOffset: Int, length: Int): Int {
        if (offset == bytes.size) return -1
        val count = minOf(length, maximumChunk, bytes.size - offset)
        bytes.copyInto(target, targetOffset, offset, offset + count)
        offset += count
        return count
    }
}

private fun String.hexBytes(): ByteArray {
    require(length % 2 == 0)
    return chunked(2).map { it.toInt(16).toByte() }.toByteArray()
}

private fun ByteArray.hex(): String = joinToString("") { "%02x".format(it.toInt() and 0xff) }
