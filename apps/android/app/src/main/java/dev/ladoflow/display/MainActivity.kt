package dev.ladoflow.display

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import dev.ladoflow.display.ui.LadoFlowApp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val transport = (application as LadoFlowApplication).usbAccessoryTransport
        transport.handleIntent(intent)
        enableEdgeToEdge()
        setContent {
            LadoFlowApp(usbTransport = transport)
        }
    }

    override fun onNewIntent(intent: android.content.Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        (application as LadoFlowApplication).usbAccessoryTransport.handleIntent(intent)
    }
}
