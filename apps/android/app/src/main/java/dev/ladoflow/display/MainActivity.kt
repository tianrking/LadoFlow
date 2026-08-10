package dev.ladoflow.display

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.ladoflow.display.ui.LadoFlowApp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val ladoFlowApplication = application as LadoFlowApplication
        ladoFlowApplication.usbAccessoryTransport.handleIntent(intent)
        enableEdgeToEdge()
        setContent {
            LadoFlowApp(
                displaySession = ladoFlowApplication.displaySession,
                wiredTransport = ladoFlowApplication.wiredTransport,
                startupFailure = ladoFlowApplication.startupFailure,
                capabilityEvidence = ladoFlowApplication.capabilityEvidence,
            )
        }
    }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        (application as LadoFlowApplication).usbAccessoryTransport.handleIntent(intent)
    }
}
