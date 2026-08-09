package dev.ladoflow.display.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.v2.createComposeRule
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
}
