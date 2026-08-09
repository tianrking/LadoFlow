package dev.ladoflow.display

import android.app.Application
import androidx.lifecycle.ProcessLifecycleOwner
import dev.ladoflow.display.transport.usb.AndroidUsbAccessoryTransport

class LadoFlowApplication : Application() {
    lateinit var usbAccessoryTransport: AndroidUsbAccessoryTransport
        private set

    override fun onCreate() {
        super.onCreate()
        usbAccessoryTransport = AndroidUsbAccessoryTransport(this)
        usbAccessoryTransport.start()
        ProcessLifecycleOwner.get().lifecycle.addObserver(usbAccessoryTransport)
    }
}
