package dev.ladoflow.display.transport.usb

const val LADO_FLOW_ACCESSORY_MANUFACTURER: String = "LadoFlow"
const val LADO_FLOW_ACCESSORY_MODEL: String = "LadoFlow Host"
const val USB_READ_BUFFER_BYTES: Int = 64 * 1024

data class UsbAccessoryIdentity(
    val manufacturer: String?,
    val model: String?,
    val description: String?,
    val version: String?,
    val serial: String?,
) {
    val displayName: String
        get() = description?.takeIf { it.isNotBlank() }
            ?: model?.takeIf { it.isNotBlank() }
            ?: "LadoFlow USB host"

    val isLadoFlowHost: Boolean
        get() = manufacturer == LADO_FLOW_ACCESSORY_MANUFACTURER &&
            model == LADO_FLOW_ACCESSORY_MODEL
}

data class UsbReconnectPolicy(
    val initialDelayMillis: Long = 250,
    val maximumDelayMillis: Long = 5_000,
    val maximumAttempts: Int = 6,
) {
    init {
        require(initialDelayMillis > 0)
        require(maximumDelayMillis >= initialDelayMillis)
        require(maximumAttempts > 0)
    }

    fun shouldRetry(attempt: Int): Boolean = attempt in 1..maximumAttempts

    fun delayMillis(attempt: Int): Long {
        require(attempt >= 1)
        var delay = initialDelayMillis
        repeat((attempt - 1).coerceAtMost(62)) {
            delay = (delay * 2).coerceAtMost(maximumDelayMillis)
        }
        return delay
    }
}

sealed interface UsbTransportState {
    data object Stopped : UsbTransportState

    data object WaitingForAccessory : UsbTransportState

    data class AwaitingPermission(val accessory: UsbAccessoryIdentity) : UsbTransportState

    data class Opening(val accessory: UsbAccessoryIdentity) : UsbTransportState

    data class Connected(val accessory: UsbAccessoryIdentity) : UsbTransportState

    data class Recovering(
        val accessory: UsbAccessoryIdentity,
        val attempt: Int,
        val delayMillis: Long,
        val reason: String,
    ) : UsbTransportState

    data class Error(
        val accessory: UsbAccessoryIdentity?,
        val reason: String,
        val retryable: Boolean,
    ) : UsbTransportState

    data class Unsupported(val reason: String) : UsbTransportState
}
