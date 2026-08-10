package dev.ladoflow.display.transport

import dev.ladoflow.display.protocol.LdflFrame
import dev.ladoflow.display.transport.tether.AndroidUsbTetherTransport
import dev.ladoflow.display.transport.tether.UsbTetherPairingState
import dev.ladoflow.display.transport.usb.AndroidUsbAccessoryTransport
import dev.ladoflow.display.transport.usb.LdflDisplayTransport
import dev.ladoflow.display.transport.usb.UsbTransportState
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineName
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.stateIn

enum class WiredTransportMode {
    Accessory,
    UsbTether,
}

/** Selects exactly one Android wired transport while preserving one LDFL session boundary. */
class AndroidWiredDisplayTransport(
    val accessory: AndroidUsbAccessoryTransport,
    val tether: AndroidUsbTetherTransport,
    parentScope: CoroutineScope,
) : LdflDisplayTransport, Closeable {
    private val transportJob = SupervisorJob(parentScope.coroutineContext[Job])
    private val scope = CoroutineScope(
        parentScope.coroutineContext + transportJob + CoroutineName("LadoFlow wired transport"),
    )
    private val closed = AtomicBoolean(false)
    private val mutableMode = MutableStateFlow(WiredTransportMode.Accessory)
    private val active = MutableStateFlow<LdflDisplayTransport>(accessory)

    val mode: StateFlow<WiredTransportMode> = mutableMode.asStateFlow()
    val tetherPairingState: StateFlow<UsbTetherPairingState> = tether.pairingState

    @OptIn(ExperimentalCoroutinesApi::class)
    override val state: StateFlow<UsbTransportState> = active
        .flatMapLatest { it.state }
        .stateIn(scope, SharingStarted.Eagerly, UsbTransportState.Stopped)

    @OptIn(ExperimentalCoroutinesApi::class)
    override val frames: Flow<LdflFrame> = active.flatMapLatest { it.frames }

    fun startUsbTetherPairing() {
        if (closed.get()) return
        mutableMode.value = WiredTransportMode.UsbTether
        active.value = tether
        accessory.disconnect()
        tether.startPairing()
    }

    fun stopUsbTetherPairing() {
        if (closed.get() || mutableMode.value != WiredTransportMode.UsbTether) return
        tether.disconnect()
    }

    fun useUsbAccessory() {
        if (closed.get()) return
        tether.disconnect()
        mutableMode.value = WiredTransportMode.Accessory
        active.value = accessory
        accessory.retry()
    }

    override suspend fun sendControl(frame: LdflFrame): Boolean =
        active.value.sendControl(frame)

    override fun trySendControl(frame: LdflFrame): Boolean =
        active.value.trySendControl(frame)

    override fun retry() {
        active.value.retry()
    }

    override fun disconnect() {
        active.value.disconnect()
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        accessory.close()
        tether.close()
        transportJob.cancel()
    }
}
