package dev.ladoflow.display.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import dev.ladoflow.display.transport.WiredTransportMode
import dev.ladoflow.display.transport.tether.TetherPairingCode
import dev.ladoflow.display.transport.tether.UsbTetherPairingState
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test

class LadoFlowUiSmokeTest {
    @get:Rule
    val composeRule = createComposeRule()

    @Test
    fun waitingForAccessoryExplainsTheUsbFirstPath() {
        composeRule.setContent {
            LadoFlowApp(
                state = DisplayUiState(stage = ConnectionStage.WaitingForAccessory),
                onEvent = {},
            )
        }

        composeRule.onNodeWithText("Connect your computer").assertIsDisplayed()
        composeRule.onNodeWithText("Check USB connection").assertIsDisplayed()
    }

    @Test
    fun failedSessionShowsTheConcreteDiagnostic() {
        composeRule.setContent {
            LadoFlowApp(
                state = DisplayUiState(
                    stage = ConnectionStage.Error,
                    lastError = "H.264 Main decoder unavailable",
                ),
                onEvent = {},
            )
        }

        composeRule.onNodeWithText("Connection needs attention").assertIsDisplayed()
        composeRule.onNodeWithText("H.264 Main decoder unavailable").assertIsDisplayed()
    }

    @Test
    fun waitingForAuthorizationIsExplicit() {
        composeRule.setContent {
            LadoFlowApp(
                state = DisplayUiState(stage = ConnectionStage.WaitingForPermission),
                onEvent = {},
            )
        }

        composeRule.onAllNodesWithText("Waiting for authorization")[0].assertIsDisplayed()
    }

    @Test
    fun connectedAndReconnectingStatesAreExplicit() {
        var stage by androidx.compose.runtime.mutableStateOf(ConnectionStage.Connected)
        composeRule.setContent {
            LadoFlowApp(state = DisplayUiState(stage = stage), onEvent = {})
        }

        composeRule.onAllNodesWithText("Connected")[0].assertIsDisplayed()
        composeRule.runOnUiThread { stage = ConnectionStage.Recovering }
        composeRule.onAllNodesWithText("Reconnecting")[0].assertIsDisplayed()
    }

    @Test
    fun protocolErrorAndDeviceDetachAreExplicit() {
        var stage by androidx.compose.runtime.mutableStateOf(ConnectionStage.ProtocolError)
        composeRule.setContent {
            LadoFlowApp(state = DisplayUiState(stage = stage), onEvent = {})
        }

        composeRule.onAllNodesWithText("Protocol error")[0].assertIsDisplayed()
        composeRule.runOnUiThread { stage = ConnectionStage.DeviceDisconnected }
        composeRule.onAllNodesWithText("Device disconnected")[0].assertIsDisplayed()
    }

    @Test
    fun usbTetherListenerShowsOneTimeCodeExpiryAndUnencryptedBoundary() {
        var event: DisplayEvent? = null
        composeRule.setContent {
            LadoFlowApp(
                state = DisplayUiState(stage = ConnectionStage.WaitingForTetherHost),
                transportMode = WiredTransportMode.UsbTether,
                tetherPairingState = UsbTetherPairingState.Listening(
                    address = "192.168.42.129",
                    port = 49_231,
                    code = TetherPairingCode("000G-40R4-0M30-E209"),
                    expiresAfterSeconds = 120,
                    failedHandshakes = 0,
                    maximumFailedHandshakes = 3,
                ),
                onEvent = { event = it },
            )
        }

        composeRule.onNodeWithText("000G-40R4-0M30-E209").assertIsDisplayed()
        composeRule.onNodeWithText("192.168.42.129:49231").assertIsDisplayed()
        composeRule.onNodeWithText("2 minutes after creation").assertIsDisplayed()
        composeRule.onNodeWithText("does not encrypt", substring = true).assertIsDisplayed()
        composeRule.onNodeWithText("Stop wired fallback").performScrollTo().performClick()
        composeRule.runOnIdle { assertEquals(DisplayEvent.StopUsbTetherRequested, event) }
    }

    @Test
    fun accessoryWaitingScreenOffersExplicitUsbTetherFallbackAction() {
        var event: DisplayEvent? = null
        composeRule.setContent {
            LadoFlowApp(
                state = DisplayUiState(stage = ConnectionStage.WaitingForAccessory),
                onEvent = { event = it },
            )
        }

        composeRule.onNodeWithText("Use USB tethering fallback").performScrollTo().performClick()
        composeRule.runOnIdle { assertEquals(DisplayEvent.StartUsbTetherRequested, event) }
    }
}
