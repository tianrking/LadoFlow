package dev.ladoflow.display.transport.usb

import android.os.ParcelFileDescriptor
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.atomic.AtomicBoolean

internal class AndroidUsbDuplexConnection private constructor(
    override val input: InputStream,
    override val output: OutputStream,
) : UsbDuplexConnection {
    private val closed = AtomicBoolean(false)

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        runCatching { input.close() }
        runCatching { output.close() }
    }

    companion object {
        fun from(descriptor: ParcelFileDescriptor): AndroidUsbDuplexConnection {
            var inputDescriptor: ParcelFileDescriptor? = null
            var outputDescriptor: ParcelFileDescriptor? = null
            try {
                inputDescriptor = ParcelFileDescriptor.dup(descriptor.fileDescriptor)
                outputDescriptor = ParcelFileDescriptor.dup(descriptor.fileDescriptor)
                return AndroidUsbDuplexConnection(
                    input = ParcelFileDescriptor.AutoCloseInputStream(inputDescriptor),
                    output = ParcelFileDescriptor.AutoCloseOutputStream(outputDescriptor),
                )
            } catch (exception: Exception) {
                runCatching { inputDescriptor?.close() }
                runCatching { outputDescriptor?.close() }
                throw exception
            } finally {
                descriptor.close()
            }
        }
    }
}
