package dev.ladoflow.display

import android.app.Application
import androidx.lifecycle.ProcessLifecycleOwner
import dev.ladoflow.display.media.AndroidDisplayCapabilityEvidence
import dev.ladoflow.display.media.AndroidMediaCodecVideoDecoder
import dev.ladoflow.display.media.probeAndroidDisplayCapabilities
import dev.ladoflow.display.session.AndroidDisplaySession
import dev.ladoflow.display.transport.AndroidWiredDisplayTransport
import dev.ladoflow.display.transport.tether.AndroidUsbTetherTransport
import dev.ladoflow.display.transport.usb.AndroidUsbAccessoryTransport
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

class LadoFlowApplication : Application() {
    lateinit var usbAccessoryTransport: AndroidUsbAccessoryTransport
        private set
    lateinit var usbTetherTransport: AndroidUsbTetherTransport
        private set
    lateinit var wiredTransport: AndroidWiredDisplayTransport
        private set
    var displaySession: AndroidDisplaySession? = null
        private set
    var capabilityEvidence: AndroidDisplayCapabilityEvidence? = null
        private set
    var startupFailure: String? = null
        private set

    private val applicationScope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)

    override fun onCreate() {
        super.onCreate()
        usbAccessoryTransport = AndroidUsbAccessoryTransport(this)
        usbTetherTransport = AndroidUsbTetherTransport()
        wiredTransport = AndroidWiredDisplayTransport(
            accessory = usbAccessoryTransport,
            tether = usbTetherTransport,
            parentScope = applicationScope,
        )
        runCatching { probeAndroidDisplayCapabilities(this) }
            .onSuccess { evidence ->
                capabilityEvidence = evidence
                val decoder = AndroidMediaCodecVideoDecoder()
                displaySession = AndroidDisplaySession(
                    transport = wiredTransport,
                    decoder = decoder,
                    localCapabilities = evidence.capabilities,
                    parentScope = applicationScope,
                ).also(AndroidDisplaySession::start)
            }
            .onFailure { exception ->
                startupFailure = exception.message
                    ?: "Unable to query an H.264 Main decoder for this display"
            }
        usbAccessoryTransport.start()
        usbTetherTransport.start()
        if (startupFailure != null) wiredTransport.disconnect()
        ProcessLifecycleOwner.get().lifecycle.addObserver(usbAccessoryTransport)
        ProcessLifecycleOwner.get().lifecycle.addObserver(usbTetherTransport)
    }
}
