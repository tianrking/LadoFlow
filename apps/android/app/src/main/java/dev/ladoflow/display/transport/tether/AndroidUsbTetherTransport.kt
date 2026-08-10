package dev.ladoflow.display.transport.tether

import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.transport.usb.LdflDisplayTransport
import dev.ladoflow.display.transport.usb.UsbDuplexConnection
import dev.ladoflow.display.transport.usb.UsbIoSession
import dev.ladoflow.display.transport.usb.UsbIoSessionState
import dev.ladoflow.display.transport.usb.UsbTransportState
import java.io.Closeable
import java.io.InputStream
import java.io.OutputStream
import java.net.Socket
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.yield

sealed interface UsbTetherPairingState {
    data object Inactive : UsbTetherPairingState

    data object Starting : UsbTetherPairingState

    data class Unavailable(val reason: String) : UsbTetherPairingState

    data class Listening(
        val address: String,
        val port: Int,
        val code: TetherPairingCode,
        val expiresAfterSeconds: Int,
        val failedHandshakes: Int,
        val maximumFailedHandshakes: Int,
        val lastFailure: String? = null,
    ) : UsbTetherPairingState

    data class Authenticating(
        val address: String,
        val port: Int,
        val code: TetherPairingCode,
        val hostAddress: String,
        val failedHandshakes: Int,
    ) : UsbTetherPairingState

    data class Authenticated(
        val address: String,
        val port: Int,
        val hostAddress: String,
    ) : UsbTetherPairingState

    data class Expired(val address: String, val port: Int) : UsbTetherPairingState

    data class LockedOut(val address: String, val port: Int) : UsbTetherPairingState

    data class Failed(val reason: String, val retryable: Boolean) : UsbTetherPairingState
}

/**
 * Explicit, foreground-only fallback for USB tethering. The pairing listener is never opened by
 * lifecycle entry alone and binds only to an interface selected by [discoverUsbTetherAddress].
 */
class AndroidUsbTetherTransport(
    private val port: Int = DEFAULT_USB_TETHER_PORT,
) : DefaultLifecycleObserver, Closeable, LdflDisplayTransport {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val mutableState = MutableStateFlow<UsbTransportState>(UsbTransportState.Stopped)
    private val mutablePairingState = MutableStateFlow<UsbTetherPairingState>(
        UsbTetherPairingState.Inactive,
    )
    private val mutableSession = MutableStateFlow<UsbIoSession?>(null)
    private val started = AtomicBoolean(false)
    private val closed = AtomicBoolean(false)
    private val generation = AtomicLong(0)
    private val resourceLock = Any()
    private val server = AtomicReference<TetherPairingServer?>(null)
    private val token = AtomicReference<TetherPairingToken?>(null)

    @Volatile
    private var foreground = false
    private var listenerJob: Job? = null
    private var sessionStateJob: Job? = null

    override val state: StateFlow<UsbTransportState> = mutableState.asStateFlow()
    val pairingState: StateFlow<UsbTetherPairingState> = mutablePairingState.asStateFlow()

    @OptIn(ExperimentalCoroutinesApi::class)
    override val frames: Flow<LdflFrame> = mutableSession
        .filterNotNull()
        .flatMapLatest { it.frames }

    init {
        require(port in 1..65_535) { "USB tether TCP port must be between 1 and 65535" }
    }

    fun start() {
        started.compareAndSet(false, true)
    }

    override fun onStart(owner: LifecycleOwner) {
        foreground = true
    }

    override fun onStop(owner: LifecycleOwner) {
        foreground = false
        stopPairingAndSession(UsbTetherPairingState.Inactive)
    }

    fun startPairing() {
        if (closed.get()) return
        if (!started.get()) start()
        if (!foreground) {
            stopPairingAndSession(
                UsbTetherPairingState.Unavailable(
                    "Keep LadoFlow in the foreground before starting USB tether pairing.",
                ),
            )
            return
        }

        val currentGeneration = beginNewGeneration()
        mutablePairingState.value = UsbTetherPairingState.Starting
        mutableState.value = UsbTransportState.Stopped
        listenerJob = scope.launch {
            val outcome = try {
                withContext(Dispatchers.IO) {
                    openAndServe(currentGeneration)
                }
            } catch (cancelled: CancellationException) {
                throw cancelled
            } catch (exception: Exception) {
                ListenerOutcome.Failed(exception.message ?: "Unable to start USB tether listener")
            }
            if (generation.get() != currentGeneration || closed.get()) {
                (outcome as? ListenerOutcome.Server)?.result
                    ?.let { it as? TetherPairingServerResult.Authenticated }
                    ?.socket
                    ?.let { socket -> runCatching { socket.close() } }
                return@launch
            }
            handleListenerOutcome(currentGeneration, outcome)
        }
    }

    override fun retry() {
        startPairing()
    }

    override fun disconnect() {
        stopPairingAndSession(UsbTetherPairingState.Inactive)
    }

    override suspend fun sendControl(frame: LdflFrame): Boolean {
        val session = mutableSession.value ?: return false
        session.sendControl(frame)
        return true
    }

    override fun trySendControl(frame: LdflFrame): Boolean =
        mutableSession.value?.trySendControl(frame) == true

    private fun openAndServe(currentGeneration: Long): ListenerOutcome {
        val address = try {
            discoverUsbTetherAddress()
        } catch (exception: Exception) {
            return ListenerOutcome.Unavailable(
                exception.message ?: "Unable to inspect Android network interfaces",
            )
        } ?: return ListenerOutcome.Unavailable(
            "No active USB tether interface was found. Enable USB tethering, keep the cable " +
                "connected, then request a new pairing code.",
        )

        val pairingToken = TetherPairingToken.generate()
        val pairingServer = synchronized(resourceLock) {
            if (generation.get() != currentGeneration || closed.get()) {
                pairingToken.invalidate()
                return ListenerOutcome.Server(TetherPairingServerResult.Closed)
            }
            token.set(pairingToken)
            try {
                TetherPairingServer.open(address, port, pairingToken).also(server::set)
            } catch (exception: Exception) {
                token.compareAndSet(pairingToken, null)
                pairingToken.invalidate()
                return ListenerOutcome.Failed(
                    exception.message ?: "Unable to bind the USB tether pairing listener",
                )
            }
        }
        if (generation.get() != currentGeneration || closed.get()) {
            server.compareAndSet(pairingServer, null)
            pairingServer.close()
            token.compareAndSet(pairingToken, null)
            return ListenerOutcome.Server(TetherPairingServerResult.Closed)
        }

        var lastFailure: String? = null
        val result = pairingServer.serve { event ->
            if (generation.get() != currentGeneration || closed.get()) return@serve
            when (event) {
                is TetherPairingServerEvent.Listening -> {
                    mutablePairingState.value = UsbTetherPairingState.Listening(
                        address = event.endpoint.address,
                        port = event.endpoint.port,
                        code = event.code,
                        expiresAfterSeconds =
                            (USB_TETHER_PAIRING_LIFETIME_MILLIS / 1_000).toInt(),
                        failedHandshakes = event.failedHandshakes,
                        maximumFailedHandshakes = USB_TETHER_MAX_FAILED_HANDSHAKES,
                        lastFailure = lastFailure,
                    )
                    mutableState.value = UsbTransportState.TetherListening(
                        address = event.endpoint.address,
                        port = event.endpoint.port,
                        failedHandshakes = event.failedHandshakes,
                    )
                }

                is TetherPairingServerEvent.Authenticating -> {
                    mutablePairingState.value = UsbTetherPairingState.Authenticating(
                        address = event.endpoint.address,
                        port = event.endpoint.port,
                        code = event.code,
                        hostAddress = event.hostAddress,
                        failedHandshakes = event.failedHandshakes,
                    )
                    mutableState.value = UsbTransportState.TetherAuthenticating(event.hostAddress)
                }

                is TetherPairingServerEvent.Rejected -> {
                    lastFailure = event.reason
                }
            }
        }
        server.compareAndSet(pairingServer, null)
        token.compareAndSet(pairingToken, null)
        if (generation.get() != currentGeneration || closed.get()) {
            (result as? TetherPairingServerResult.Authenticated)?.socket?.let { socket ->
                runCatching { socket.close() }
            }
            return ListenerOutcome.Server(TetherPairingServerResult.Closed)
        }
        return ListenerOutcome.Server(result)
    }

    private suspend fun handleListenerOutcome(
        currentGeneration: Long,
        outcome: ListenerOutcome,
    ) {
        when (outcome) {
            is ListenerOutcome.Unavailable -> {
                mutablePairingState.value = UsbTetherPairingState.Unavailable(outcome.reason)
                mutableState.value = UsbTransportState.Error(
                    accessory = null,
                    reason = outcome.reason,
                    retryable = true,
                )
            }

            is ListenerOutcome.Failed -> {
                mutablePairingState.value = UsbTetherPairingState.Failed(
                    outcome.reason,
                    retryable = true,
                )
                mutableState.value = UsbTransportState.Error(
                    accessory = null,
                    reason = outcome.reason,
                    retryable = true,
                )
            }

            is ListenerOutcome.Server -> when (val result = outcome.result) {
                is TetherPairingServerResult.Authenticated -> {
                    activateAuthenticatedSocket(currentGeneration, result)
                }

                is TetherPairingServerResult.Expired -> {
                    val reason = "USB tether pairing code expired after two minutes."
                    mutablePairingState.value = UsbTetherPairingState.Expired(
                        result.endpoint.address,
                        result.endpoint.port,
                    )
                    mutableState.value = UsbTransportState.Error(null, reason, retryable = true)
                }

                is TetherPairingServerResult.LockedOut -> {
                    val reason = "USB tether pairing stopped after three failed handshakes."
                    mutablePairingState.value = UsbTetherPairingState.LockedOut(
                        result.endpoint.address,
                        result.endpoint.port,
                    )
                    mutableState.value = UsbTransportState.Error(null, reason, retryable = true)
                }

                TetherPairingServerResult.Closed -> {
                    mutablePairingState.value = UsbTetherPairingState.Inactive
                    mutableState.value = UsbTransportState.Stopped
                }

                is TetherPairingServerResult.Failed -> {
                    mutablePairingState.value = UsbTetherPairingState.Failed(
                        result.reason,
                        retryable = true,
                    )
                    mutableState.value = UsbTransportState.Error(
                        null,
                        result.reason,
                        retryable = true,
                    )
                }
            }
        }
    }

    private suspend fun activateAuthenticatedSocket(
        currentGeneration: Long,
        result: TetherPairingServerResult.Authenticated,
    ) {
        if (generation.get() != currentGeneration || closed.get() || !foreground) {
            runCatching { result.socket.close() }
            return
        }
        val connection = SocketDuplexConnection(result.socket)
        val ioSession = UsbIoSession(connection, scope)
        closeActiveSession()
        mutableSession.value = ioSession
        mutablePairingState.value = UsbTetherPairingState.Authenticated(
            address = result.endpoint.address,
            port = result.endpoint.port,
            hostAddress = result.hostAddress,
        )
        mutableState.value = UsbTransportState.TetherConnected(result.hostAddress)

        // Give the display session a chance to enqueue its LDFL Hello/Capabilities before the
        // reader consumes any Host bytes that were pipelined behind DisplayFinished.
        yield()
        if (generation.get() != currentGeneration || mutableSession.value !== ioSession) {
            ioSession.close()
            return
        }
        ioSession.start()
        sessionStateJob = scope.launch {
            ioSession.state.collect { sessionState ->
                if (
                    sessionState is UsbIoSessionState.Failed &&
                    generation.get() == currentGeneration &&
                    mutableSession.value === ioSession
                ) {
                    mutableSession.value = null
                    val reason = "USB tether LDFL stream ended: ${sessionState.message}"
                    mutablePairingState.value = UsbTetherPairingState.Failed(
                        reason,
                        retryable = true,
                    )
                    mutableState.value = UsbTransportState.Error(
                        accessory = null,
                        reason = reason,
                        retryable = true,
                    )
                }
            }
        }
    }

    private fun beginNewGeneration(): Long {
        return synchronized(resourceLock) {
            val next = generation.incrementAndGet()
            listenerJob?.cancel()
            listenerJob = null
            server.getAndSet(null)?.close()
            token.getAndSet(null)?.invalidate()
            closeActiveSession()
            next
        }
    }

    private fun stopPairingAndSession(finalState: UsbTetherPairingState) {
        beginNewGeneration()
        mutablePairingState.value = finalState
        mutableState.value = UsbTransportState.Stopped
    }

    private fun closeActiveSession() {
        sessionStateJob?.cancel()
        sessionStateJob = null
        val activeSession = mutableSession.value
        mutableSession.value = null
        activeSession?.close()
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        foreground = false
        stopPairingAndSession(UsbTetherPairingState.Inactive)
        scope.coroutineContext[Job]?.cancel()
    }
}

private sealed interface ListenerOutcome {
    data class Server(val result: TetherPairingServerResult) : ListenerOutcome

    data class Unavailable(val reason: String) : ListenerOutcome

    data class Failed(val reason: String) : ListenerOutcome
}

private class SocketDuplexConnection(
    private val socket: Socket,
) : UsbDuplexConnection {
    override val input: InputStream = socket.getInputStream()
    override val output: OutputStream = socket.getOutputStream()

    override fun close() {
        runCatching { socket.close() }
    }
}
