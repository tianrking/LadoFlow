package dev.ladoflow.display

import android.app.Application
import androidx.lifecycle.ProcessLifecycleOwner
import dev.ladoflow.display.media.AndroidDisplayCapabilityEvidence
import dev.ladoflow.display.media.AndroidMediaCodecVideoDecoder
import dev.ladoflow.display.media.probeAndroidDisplayCapabilities
import dev.ladoflow.display.session.AndroidDisplaySession
import dev.ladoflow.display.transport.usb.AndroidUsbAccessoryTransport
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob

class LadoFlowApplication : Application() {
    lateinit var usbAccessoryTransport: AndroidUsbAccessoryTransport
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
        runCatching { probeAndroidDisplayCapabilities(this) }
            .onSuccess { evidence ->
                capabilityEvidence = evidence
                val decoder = AndroidMediaCodecVideoDecoder()
                displaySession = AndroidDisplaySession(
                    transport = usbAccessoryTransport,
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
        if (startupFailure != null) usbAccessoryTransport.disconnect()
        ProcessLifecycleOwner.get().lifecycle.addObserver(usbAccessoryTransport)
    }
}
