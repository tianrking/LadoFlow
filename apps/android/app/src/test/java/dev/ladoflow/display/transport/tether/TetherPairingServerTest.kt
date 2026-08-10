package dev.ladoflow.display.transport.tether

import java.io.InputStream
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.util.concurrent.Executors
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class TetherPairingServerTest {
    private val loopback = InetAddress.getLoopbackAddress()
    private val tokenBytes = byteArrayOf(0, 1, 2, 3, 4, 5, 6, 7, 8, 9)
    private val hostNonce = ByteArray(16) { 0x11 }
    private val displayNonce = ByteArray(16) { 0x22 }

    @Test
    fun `authenticated socket idles between Pair and Start until explicit close`() {
        val token = TetherPairingToken.fromBytes(tokenBytes)
        val server = testServer(token)
        val executor = Executors.newSingleThreadExecutor()
        val resultFuture = executor.submit<TetherPairingServerResult> { server.serve {} }
        val client = Socket(loopback, server.endpoint.port).apply { soTimeout = 2_000 }
        try {
            val firstLdflBytes = byteArrayOf(0x4c, 0x44, 0x46, 0x4c, 1)
            client.getOutputStream().write(hostHello() + hostFinished() + firstLdflBytes)
            client.getOutputStream().flush()

            val displayBytes = client.getInputStream().readTestBytes(TETHER_PAIRING_RECORD_BYTES * 2)
            val displayInput = java.io.ByteArrayInputStream(displayBytes)
            readPairingRecord(displayInput, TetherPairingKind.DisplayHello)
            readPairingRecord(displayInput, TetherPairingKind.DisplayFinished)

            val result = resultFuture.get(3, TimeUnit.SECONDS)
            assertTrue(result is TetherPairingServerResult.Authenticated)
            val authenticated = result as TetherPairingServerResult.Authenticated
            assertEquals(server.endpoint, authenticated.endpoint)
            assertArrayEquals(
                firstLdflBytes,
                authenticated.socket.getInputStream().readTestBytes(firstLdflBytes.size),
            )
            assertEquals(0, authenticated.socket.soTimeout)
            assertFalse(token.isValid())

            val idleRead = executor.submit<Int> { authenticated.socket.getInputStream().read() }
            Thread.sleep(250)
            assertFalse("SO_TIMEOUT zero must leave the pre-Start read blocked", idleRead.isDone)
            authenticated.socket.close()
            runCatching { idleRead.get(2, TimeUnit.SECONDS) }
            assertTrue("socket close must release the blocked read", idleRead.isDone)

            assertThrows(Exception::class.java) {
                Socket().use { second ->
                    second.connect(InetSocketAddress(loopback, server.endpoint.port), 250)
                }
            }
        } finally {
            client.close()
            server.close()
            executor.shutdownNow()
        }
    }

    @Test
    fun `third failed handshake locks listener and invalidates token`() {
        val token = TetherPairingToken.fromBytes(tokenBytes)
        val server = testServer(token, lifetimeMillis = 3_000)
        val events = LinkedBlockingQueue<TetherPairingServerEvent>()
        val executor = Executors.newSingleThreadExecutor()
        val resultFuture = executor.submit<TetherPairingServerResult> { server.serve(events::offer) }
        try {
            assertTrue(events.poll(2, TimeUnit.SECONDS) is TetherPairingServerEvent.Listening)
            repeat(3) { expectedFailure ->
                Socket(loopback, server.endpoint.port).use { client ->
                    client.getOutputStream().write(ByteArray(TETHER_PAIRING_RECORD_BYTES))
                    client.getOutputStream().flush()
                }
                var rejected: TetherPairingServerEvent.Rejected? = null
                while (rejected == null) {
                    val event = events.poll(2, TimeUnit.SECONDS)
                    assertTrue("pairing server must report rejection", event != null)
                    if (event is TetherPairingServerEvent.Rejected) rejected = event
                }
                assertEquals(expectedFailure + 1, rejected.failedHandshakes)
            }

            assertTrue(resultFuture.get(3, TimeUnit.SECONDS) is TetherPairingServerResult.LockedOut)
            assertFalse(token.isValid())
        } finally {
            server.close()
            executor.shutdownNow()
        }
    }

    @Test
    fun `pairing lifetime expires and closes listener without a host`() {
        val token = TetherPairingToken.fromBytes(tokenBytes)
        val server = testServer(token, lifetimeMillis = 75)
        val executor = Executors.newSingleThreadExecutor()
        val resultFuture = executor.submit<TetherPairingServerResult> { server.serve {} }
        try {
            assertTrue(resultFuture.get(2, TimeUnit.SECONDS) is TetherPairingServerResult.Expired)
            assertFalse(token.isValid())
        } finally {
            server.close()
            executor.shutdownNow()
        }
    }

    @Test
    fun `silent peer is bounded by socket timeout and counts as one failed handshake`() {
        val token = TetherPairingToken.fromBytes(tokenBytes)
        val server = TetherPairingServer.open(
            address = loopback,
            port = 0,
            token = token,
            displayNonceSource = { displayNonce },
            lifetimeMillis = 2_000,
            handshakeTimeoutMillis = 75,
            maximumFailedHandshakes = 1,
        )
        val executor = Executors.newSingleThreadExecutor()
        val resultFuture = executor.submit<TetherPairingServerResult> { server.serve {} }
        val client = Socket(loopback, server.endpoint.port)
        try {
            assertTrue(resultFuture.get(2, TimeUnit.SECONDS) is TetherPairingServerResult.LockedOut)
            assertFalse(token.isValid())
        } finally {
            client.close()
            server.close()
            executor.shutdownNow()
        }
    }

    private fun testServer(
        token: TetherPairingToken,
        lifetimeMillis: Long = 3_000,
    ): TetherPairingServer = TetherPairingServer.open(
        address = loopback,
        port = 0,
        token = token,
        displayNonceSource = { displayNonce },
        lifetimeMillis = lifetimeMillis,
        handshakeTimeoutMillis = 500,
    )

    private fun hostHello(): ByteArray = encodePairingRecord(
        TetherPairingRecord(TetherPairingKind.HostHello, hostNonce, ByteArray(32)),
    )

    private fun hostFinished(): ByteArray = encodePairingRecord(
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
}

private fun InputStream.readTestBytes(count: Int): ByteArray {
    val bytes = ByteArray(count)
    var offset = 0
    while (offset < bytes.size) {
        val read = read(bytes, offset, bytes.size - offset)
        check(read > 0) { "stream ended after $offset of $count bytes" }
        offset += read
    }
    return bytes
}
