package dev.ladoflow.display.transport.tether

import java.io.Closeable
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketException
import java.net.SocketTimeoutException
import java.security.SecureRandom
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlin.math.min

internal const val DEFAULT_USB_TETHER_PORT: Int = 49_231
internal const val USB_TETHER_PAIRING_LIFETIME_MILLIS: Long = 120_000
internal const val USB_TETHER_MAX_FAILED_HANDSHAKES: Int = 3
internal const val USB_TETHER_HANDSHAKE_TIMEOUT_MILLIS: Int = 10_000

internal data class TetherListenerEndpoint(
    val address: String,
    val port: Int,
) {
    val displayValue: String
        get() = "$address:$port"
}

internal sealed interface TetherPairingServerEvent {
    data class Listening(
        val endpoint: TetherListenerEndpoint,
        val code: TetherPairingCode,
        val failedHandshakes: Int,
    ) : TetherPairingServerEvent

    data class Authenticating(
        val endpoint: TetherListenerEndpoint,
        val code: TetherPairingCode,
        val hostAddress: String,
        val failedHandshakes: Int,
    ) : TetherPairingServerEvent

    data class Rejected(
        val endpoint: TetherListenerEndpoint,
        val failedHandshakes: Int,
        val reason: String,
    ) : TetherPairingServerEvent
}

internal sealed interface TetherPairingServerResult {
    data class Authenticated(
        val socket: Socket,
        val endpoint: TetherListenerEndpoint,
        val hostAddress: String,
    ) : TetherPairingServerResult

    data class Expired(val endpoint: TetherListenerEndpoint) : TetherPairingServerResult

    data class LockedOut(val endpoint: TetherListenerEndpoint) : TetherPairingServerResult

    data object Closed : TetherPairingServerResult

    data class Failed(val reason: String) : TetherPairingServerResult
}

/** Blocking, single-use pairing listener. Call [serve] from an I/O dispatcher. */
internal class TetherPairingServer private constructor(
    private val serverSocket: ServerSocket,
    private val token: TetherPairingToken,
    private val displayNonceSource: () -> ByteArray,
    private val lifetimeMillis: Long,
    private val handshakeTimeoutMillis: Int,
    private val maximumFailedHandshakes: Int,
    private val monotonicNanos: () -> Long,
) : Closeable {
    private val served = AtomicBoolean(false)
    private val closed = AtomicBoolean(false)
    private val pendingSocket = AtomicReference<Socket?>(null)

    val endpoint = TetherListenerEndpoint(
        address = requireNotNull(serverSocket.inetAddress.hostAddress),
        port = serverSocket.localPort,
    )

    fun serve(onEvent: (TetherPairingServerEvent) -> Unit): TetherPairingServerResult {
        check(served.compareAndSet(false, true)) { "USB tether pairing listener is single-use" }
        val lifetimeNanos = lifetimeMillis * NANOS_PER_MILLISECOND
        val startedAt = monotonicNanos()
        val deadline = if (Long.MAX_VALUE - startedAt < lifetimeNanos) {
            Long.MAX_VALUE
        } else {
            startedAt + lifetimeNanos
        }
        var failedHandshakes = 0
        onEvent(
            TetherPairingServerEvent.Listening(
                endpoint,
                token.displayCode,
                failedHandshakes,
            ),
        )

        try {
            while (!closed.get()) {
                val remainingMillis = remainingMillis(deadline)
                if (remainingMillis <= 0) return TetherPairingServerResult.Expired(endpoint)
                serverSocket.soTimeout = min(
                    ACCEPT_POLL_MILLIS.toLong(),
                    remainingMillis,
                ).coerceAtLeast(1).toInt()
                val socket = try {
                    serverSocket.accept()
                } catch (_: SocketTimeoutException) {
                    continue
                } catch (_: SocketException) {
                    return if (closed.get()) {
                        TetherPairingServerResult.Closed
                    } else {
                        TetherPairingServerResult.Failed("USB tether listener socket closed")
                    }
                } catch (exception: Exception) {
                    return TetherPairingServerResult.Failed(
                        exception.message ?: "USB tether listener failed",
                    )
                }
                pendingSocket.set(socket)
                val hostAddress = socket.inetAddress.hostAddress ?: "USB tether host"
                val paired = runCatching {
                    socket.tcpNoDelay = true
                    socket.keepAlive = false
                    socket.soTimeout = min(
                        handshakeTimeoutMillis,
                        remainingMillis(deadline).coerceAtMost(Int.MAX_VALUE.toLong()).toInt(),
                    ).coerceAtLeast(1)
                    onEvent(
                        TetherPairingServerEvent.Authenticating(
                            endpoint,
                            token.displayCode,
                            hostAddress,
                            failedHandshakes,
                        ),
                    )
                    performDisplayPairingHandshake(
                        input = socket.getInputStream(),
                        output = socket.getOutputStream(),
                        token = token,
                        displayNonceSource = displayNonceSource,
                    )
                }
                if (paired.isSuccess) {
                    pendingSocket.compareAndSet(socket, null)
                    // Pair and Start are separate Host actions. There is no protocol heartbeat
                    // before LDFL starts, so an authenticated socket must tolerate indefinite
                    // idle time. Closing the socket remains the cancellation mechanism.
                    socket.soTimeout = 0
                    runCatching { serverSocket.close() }
                    return TetherPairingServerResult.Authenticated(
                        socket = socket,
                        endpoint = endpoint,
                        hostAddress = hostAddress,
                    )
                }

                pendingSocket.compareAndSet(socket, null)
                runCatching { socket.close() }
                if (remainingMillis(deadline) <= 0) {
                    return TetherPairingServerResult.Expired(endpoint)
                }
                failedHandshakes += 1
                val reason = paired.exceptionOrNull()?.message ?: "USB tether pairing failed"
                onEvent(
                    TetherPairingServerEvent.Rejected(
                        endpoint = endpoint,
                        failedHandshakes = failedHandshakes,
                        reason = reason,
                    ),
                )
                if (failedHandshakes >= maximumFailedHandshakes) {
                    return TetherPairingServerResult.LockedOut(endpoint)
                }
                onEvent(
                    TetherPairingServerEvent.Listening(
                        endpoint,
                        token.displayCode,
                        failedHandshakes,
                    ),
                )
            }
            return TetherPairingServerResult.Closed
        } finally {
            token.invalidate()
            runCatching { serverSocket.close() }
            pendingSocket.getAndSet(null)?.let { socket -> runCatching { socket.close() } }
        }
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        token.invalidate()
        runCatching { serverSocket.close() }
        pendingSocket.getAndSet(null)?.let { socket -> runCatching { socket.close() } }
    }

    private fun remainingMillis(deadlineNanos: Long): Long {
        val remainingNanos = deadlineNanos - monotonicNanos()
        if (remainingNanos <= 0) return 0
        return ((remainingNanos + NANOS_PER_MILLISECOND - 1) / NANOS_PER_MILLISECOND)
    }

    companion object {
        fun open(
            address: InetAddress,
            port: Int = DEFAULT_USB_TETHER_PORT,
            token: TetherPairingToken = TetherPairingToken.generate(),
            displayNonceSource: () -> ByteArray = {
                ByteArray(TETHER_PAIRING_NONCE_BYTES).also(SecureRandom()::nextBytes)
            },
            lifetimeMillis: Long = USB_TETHER_PAIRING_LIFETIME_MILLIS,
            handshakeTimeoutMillis: Int = USB_TETHER_HANDSHAKE_TIMEOUT_MILLIS,
            maximumFailedHandshakes: Int = USB_TETHER_MAX_FAILED_HANDSHAKES,
            monotonicNanos: () -> Long = System::nanoTime,
        ): TetherPairingServer {
            require(port in 0..65_535)
            require(lifetimeMillis > 0)
            require(handshakeTimeoutMillis > 0)
            require(maximumFailedHandshakes > 0)
            val socket = ServerSocket()
            try {
                socket.reuseAddress = false
                socket.bind(InetSocketAddress(address, port), 1)
                return TetherPairingServer(
                    serverSocket = socket,
                    token = token,
                    displayNonceSource = displayNonceSource,
                    lifetimeMillis = lifetimeMillis,
                    handshakeTimeoutMillis = handshakeTimeoutMillis,
                    maximumFailedHandshakes = maximumFailedHandshakes,
                    monotonicNanos = monotonicNanos,
                )
            } catch (exception: Exception) {
                token.invalidate()
                runCatching { socket.close() }
                throw exception
            }
        }
    }
}

private const val ACCEPT_POLL_MILLIS = 1_000
private const val NANOS_PER_MILLISECOND = 1_000_000L
