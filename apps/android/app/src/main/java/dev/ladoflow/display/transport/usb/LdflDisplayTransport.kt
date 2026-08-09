package dev.ladoflow.display.transport.usb

import dev.ladoflow.display.protocol.LdflFrame
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow

/** Testable LDFL boundary exposed by the Android accessory transport. */
interface LdflDisplayTransport {
    val state: StateFlow<UsbTransportState>
    /** Raw USB wire order, before control/media dispatch. */
    val frames: Flow<LdflFrame>

    suspend fun sendControl(frame: LdflFrame): Boolean

    fun trySendControl(frame: LdflFrame): Boolean

    fun retry()

    fun disconnect()
}
