package dev.ladoflow.display.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DisplayStateMachineTest {
    @Test
    fun `happy path reaches displaying with negotiated configuration`() {
        val configuration = StreamConfiguration(
            width = 1920,
            height = 1080,
            refreshRateHz = 60,
            codec = "H.264 High",
        )

        val attached = DisplayStateMachine.reduce(
            DisplayUiState(),
            DisplayEvent.AccessoryAttached("LadoFlow Host"),
        )
        assertEquals(ConnectionStage.WaitingForPermission, attached.stage)

        val pairing = DisplayStateMachine.reduce(attached, DisplayEvent.PermissionGranted)
        assertEquals(ConnectionStage.Pairing, pairing.stage)

        val connected = DisplayStateMachine.reduce(
            pairing,
            DisplayEvent.PairingCompleted("Studio PC"),
        )
        assertEquals(ConnectionStage.Connected, connected.stage)
        assertEquals("Studio PC", connected.hostName)

        val displaying = DisplayStateMachine.reduce(
            connected,
            DisplayEvent.StreamStarted(configuration),
        )
        assertEquals(ConnectionStage.Displaying, displaying.stage)
        assertEquals(configuration, displaying.stream)
        assertNull(displaying.lastError)
    }

    @Test
    fun `interruption is recoverable without discarding host identity`() {
        val connected = DisplayUiState(
            stage = ConnectionStage.Displaying,
            hostName = "Laptop",
            stream = StreamConfiguration(1280, 800, 60, "H.264 Main"),
        )

        val interrupted = DisplayStateMachine.reduce(
            connected,
            DisplayEvent.LinkInterrupted("Accessory detached"),
        )
        assertEquals(ConnectionStage.Recovering, interrupted.stage)
        assertEquals(1, interrupted.recoveryAttempt)
        assertEquals("Laptop", interrupted.hostName)

        val recovered = DisplayStateMachine.reduce(interrupted, DisplayEvent.RecoverySucceeded)
        assertEquals(ConnectionStage.Connected, recovered.stage)
        assertEquals("Laptop", recovered.hostName)
        assertNull(recovered.lastError)
    }

    @Test
    fun `disconnect clears session data but preserves local preferences`() {
        val preferences = DisplayPreferences(
            qualityMode = QualityMode.LowLatency,
            keepScreenAwake = false,
            showRemotePointer = false,
            showDiagnosticsOverlay = true,
        )
        val active = DisplayUiState(
            stage = ConnectionStage.Displaying,
            hostName = "Workstation",
            stream = StreamConfiguration(2560, 1600, 60, "H.264 High"),
            preferences = preferences,
        )

        val disconnected = DisplayStateMachine.reduce(active, DisplayEvent.Disconnected)

        assertEquals(ConnectionStage.Disconnected, disconnected.stage)
        assertNull(disconnected.hostName)
        assertNull(disconnected.stream)
        assertEquals(preferences, disconnected.preferences)
    }

    @Test
    fun `permission denial exposes retryable error state`() {
        val waiting = DisplayUiState(stage = ConnectionStage.WaitingForPermission)

        val denied = DisplayStateMachine.reduce(
            waiting,
            DisplayEvent.PermissionDenied("User denied Android USB permission"),
        )
        assertEquals(ConnectionStage.Error, denied.stage)
        assertEquals("User denied Android USB permission", denied.lastError)

        val retried = DisplayStateMachine.reduce(denied, DisplayEvent.RetryRequested)
        assertEquals(ConnectionStage.WaitingForAccessory, retried.stage)
        assertNull(retried.lastError)
    }

    @Test
    fun `open USB link waits for protocol pairing instead of claiming a session`() {
        val linked = DisplayStateMachine.reduce(
            DisplayUiState(stage = ConnectionStage.WaitingForPermission),
            DisplayEvent.UsbLinkConnected("Studio host accessory"),
        )

        assertEquals(ConnectionStage.Pairing, linked.stage)
        assertEquals("Studio host accessory", linked.accessoryName)
        assertNull(linked.hostName)
    }
}
