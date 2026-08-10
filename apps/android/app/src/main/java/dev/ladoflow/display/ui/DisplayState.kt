package dev.ladoflow.display.ui

enum class ConnectionStage {
    Disconnected,
    DeviceDisconnected,
    WaitingForAccessory,
    WaitingForTetherHost,
    WaitingForPermission,
    Pairing,
    Connected,
    Displaying,
    Recovering,
    ProtocolError,
    Error,
}

enum class QualityMode {
    Automatic,
    Quality,
    Balanced,
    LowLatency,
}

data class DisplayPreferences(
    val qualityMode: QualityMode = QualityMode.Automatic,
    val keepScreenAwake: Boolean = true,
    val showRemotePointer: Boolean = false,
    val showDiagnosticsOverlay: Boolean = false,
)

data class StreamConfiguration(
    val width: Int,
    val height: Int,
    val refreshRateHz: Int,
    val codec: String,
)

data class StreamMetrics(
    val releasedToSurfaceFrames: Long = 0,
    val droppedFrames: Long = 0,
    val queueDepth: Int = 0,
    val decodeLatencyMillis: Double? = null,
)

data class DisplayUiState(
    val stage: ConnectionStage = ConnectionStage.WaitingForAccessory,
    val hostName: String? = null,
    val accessoryName: String? = null,
    val detail: String = "Connect this device to a computer running LadoFlow Host.",
    val stream: StreamConfiguration? = null,
    val metrics: StreamMetrics = StreamMetrics(),
    val preferences: DisplayPreferences = DisplayPreferences(),
    val recoveryAttempt: Int = 0,
    val lastError: String? = null,
)

sealed interface DisplayEvent {
    data object RetryRequested : DisplayEvent

    data object StartUsbTetherRequested : DisplayEvent

    data object StopUsbTetherRequested : DisplayEvent

    data object UseUsbAccessoryRequested : DisplayEvent

    data class AccessoryAttached(val name: String?) : DisplayEvent

    data object PermissionRequested : DisplayEvent

    data object PermissionGranted : DisplayEvent

    data class TetherListenerReady(
        val address: String,
        val port: Int,
        val failedHandshakes: Int,
    ) : DisplayEvent

    data class TetherHostAuthenticating(val hostAddress: String) : DisplayEvent

    data class PermissionDenied(val reason: String) : DisplayEvent

    data class PairingCompleted(val hostName: String) : DisplayEvent

    data class UsbLinkConnected(val accessoryName: String) : DisplayEvent

    data class StreamStarted(
        val configuration: StreamConfiguration,
        val hostName: String? = null,
    ) : DisplayEvent

    data class StreamConfigured(
        val configuration: StreamConfiguration,
        val hostName: String? = null,
    ) : DisplayEvent

    data class DecoderSurfaceReady(
        val configuration: StreamConfiguration,
        val hostName: String? = null,
    ) : DisplayEvent

    data class MetricsUpdated(val metrics: StreamMetrics) : DisplayEvent

    data class LinkInterrupted(val reason: String) : DisplayEvent

    data class RecoveryAttempted(
        val attempt: Int,
        val reason: String,
    ) : DisplayEvent

    data object RecoverySucceeded : DisplayEvent

    data class Failed(val reason: String) : DisplayEvent

    data class ProtocolFailed(val reason: String) : DisplayEvent

    data class DeviceDisconnected(val accessoryName: String?) : DisplayEvent

    data object Disconnected : DisplayEvent

    data object TransportStopped : DisplayEvent

    data class PreferencesChanged(val preferences: DisplayPreferences) : DisplayEvent
}

object DisplayStateMachine {
    fun reduce(state: DisplayUiState, event: DisplayEvent): DisplayUiState = when (event) {
        DisplayEvent.StartUsbTetherRequested,
        DisplayEvent.StopUsbTetherRequested,
        DisplayEvent.UseUsbAccessoryRequested,
        -> state

        DisplayEvent.RetryRequested -> state.copy(
            stage = ConnectionStage.WaitingForAccessory,
            detail = "Waiting for a USB accessory connection.",
            hostName = null,
            stream = null,
            metrics = StreamMetrics(),
            recoveryAttempt = 0,
            lastError = null,
        )

        is DisplayEvent.AccessoryAttached -> state.copy(
            stage = ConnectionStage.WaitingForPermission,
            accessoryName = event.name,
            hostName = null,
            stream = null,
            detail = "Approve LadoFlow when Android asks to open this USB accessory.",
            recoveryAttempt = 0,
            lastError = null,
        )

        DisplayEvent.PermissionRequested -> state.copy(
            stage = ConnectionStage.WaitingForPermission,
            detail = "Waiting for Android USB permission.",
        )

        DisplayEvent.PermissionGranted -> state.copy(
            stage = ConnectionStage.Pairing,
            detail = "Verifying the local host and negotiating display capabilities.",
            lastError = null,
        )

        is DisplayEvent.TetherListenerReady -> state.copy(
            stage = ConnectionStage.WaitingForTetherHost,
            hostName = null,
            stream = null,
            detail = if (event.failedHandshakes == 0) {
                "USB tether listener ready at ${event.address}:${event.port}. Enter the code on the host."
            } else {
                "Pairing rejected ${event.failedHandshakes} time(s). The listener is still waiting."
            },
            recoveryAttempt = 0,
            lastError = null,
        )

        is DisplayEvent.TetherHostAuthenticating -> state.copy(
            stage = ConnectionStage.Pairing,
            hostName = null,
            stream = null,
            detail = "Authenticating USB tether host ${event.hostAddress} before LDFL starts.",
            recoveryAttempt = 0,
            lastError = null,
        )

        is DisplayEvent.PermissionDenied -> state.copy(
            stage = ConnectionStage.Error,
            detail = "USB permission was not granted.",
            lastError = event.reason,
        )

        is DisplayEvent.PairingCompleted -> state.copy(
            stage = ConnectionStage.Connected,
            hostName = event.hostName,
            detail = "Connected on the local USB link. Waiting for a display configuration.",
            stream = null,
            recoveryAttempt = 0,
            lastError = null,
        )

        is DisplayEvent.UsbLinkConnected -> state.copy(
            stage = ConnectionStage.Pairing,
            accessoryName = event.accessoryName,
            detail = "USB link open. Waiting for the host's LDFL Hello and capabilities.",
            recoveryAttempt = 0,
            lastError = null,
        )

        is DisplayEvent.StreamStarted -> state.copy(
            stage = ConnectionStage.Displaying,
            hostName = event.hostName ?: state.hostName,
            stream = event.configuration,
            detail = "The host is sending this extended display.",
            recoveryAttempt = 0,
            lastError = null,
        )

        is DisplayEvent.StreamConfigured -> state.copy(
            stage = ConnectionStage.Connected,
            hostName = event.hostName ?: state.hostName,
            stream = event.configuration,
            detail = "Display configuration accepted. Preparing the decoder Surface.",
            recoveryAttempt = 0,
            lastError = null,
        )

        is DisplayEvent.DecoderSurfaceReady -> state.copy(
            stage = ConnectionStage.Connected,
            hostName = event.hostName ?: state.hostName,
            stream = event.configuration,
            detail = "Decoder Surface ready. Waiting for the first H.264 keyframe output.",
            recoveryAttempt = 0,
            lastError = null,
        )

        is DisplayEvent.MetricsUpdated -> state.copy(metrics = event.metrics)

        is DisplayEvent.LinkInterrupted -> state.copy(
            stage = ConnectionStage.Recovering,
            detail = "The USB stream paused. Reconnecting with a fresh LDFL handshake.",
            recoveryAttempt = 1,
            lastError = event.reason,
        )

        is DisplayEvent.RecoveryAttempted -> state.copy(
            stage = ConnectionStage.Recovering,
            recoveryAttempt = event.attempt.coerceAtLeast(1),
            detail = "Reconnecting to the host (attempt ${event.attempt.coerceAtLeast(1)}).",
            lastError = event.reason,
        )

        DisplayEvent.RecoverySucceeded -> state.copy(
            stage = ConnectionStage.Connected,
            detail = "USB connection restored. Waiting for the next keyframe.",
            recoveryAttempt = 0,
            lastError = null,
        )

        is DisplayEvent.Failed -> state.copy(
            stage = ConnectionStage.Error,
            detail = "LadoFlow could not continue this display session.",
            stream = null,
            lastError = event.reason,
        )

        is DisplayEvent.ProtocolFailed -> state.copy(
            stage = ConnectionStage.ProtocolError,
            detail = "The host sent data that does not match the LDFL v1 session contract.",
            stream = null,
            lastError = event.reason,
        )

        is DisplayEvent.DeviceDisconnected -> DisplayUiState(
            stage = ConnectionStage.DeviceDisconnected,
            accessoryName = event.accessoryName,
            detail = "The USB accessory was detached. Reconnect the cable to start a new session.",
            preferences = state.preferences,
        )

        DisplayEvent.Disconnected -> DisplayUiState(
            stage = ConnectionStage.Disconnected,
            detail = "The host is disconnected. Your screen remains private on this device.",
            preferences = state.preferences,
        )

        DisplayEvent.TransportStopped -> DisplayUiState(
            stage = ConnectionStage.Disconnected,
            detail = "USB transport is paused. Reopen or retry LadoFlow to connect again.",
            preferences = state.preferences,
        )

        is DisplayEvent.PreferencesChanged -> state.copy(preferences = event.preferences)
    }
}
