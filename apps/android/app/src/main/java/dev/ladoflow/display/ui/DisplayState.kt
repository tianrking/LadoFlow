package dev.ladoflow.display.ui

enum class ConnectionStage {
    Disconnected,
    WaitingForAccessory,
    WaitingForPermission,
    Pairing,
    Connected,
    Displaying,
    Recovering,
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

    data class AccessoryAttached(val name: String?) : DisplayEvent

    data object PermissionRequested : DisplayEvent

    data object PermissionGranted : DisplayEvent

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

    data object Disconnected : DisplayEvent

    data object TransportStopped : DisplayEvent

    data class PreferencesChanged(val preferences: DisplayPreferences) : DisplayEvent
}

object DisplayStateMachine {
    fun reduce(state: DisplayUiState, event: DisplayEvent): DisplayUiState = when (event) {
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
            detail = "The USB stream paused. Reconnecting without discarding the session.",
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
