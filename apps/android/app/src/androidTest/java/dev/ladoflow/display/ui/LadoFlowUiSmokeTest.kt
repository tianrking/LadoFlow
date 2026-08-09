package dev.ladoflow.display.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
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
}
