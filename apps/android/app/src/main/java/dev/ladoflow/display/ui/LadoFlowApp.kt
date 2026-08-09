package dev.ladoflow.display.ui

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.BugReport
import androidx.compose.material.icons.outlined.Cable
import androidx.compose.material.icons.outlined.DisplaySettings
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Usb
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationRail
import androidx.compose.material3.NavigationRailItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.scale
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import dev.ladoflow.display.input.AndroidInputController
import dev.ladoflow.display.media.AndroidDisplayCapabilityEvidence
import dev.ladoflow.display.media.DecoderSurfaceController
import dev.ladoflow.display.media.MediaCodecSurface
import dev.ladoflow.display.protocol.DisplayConfigPayload
import dev.ladoflow.display.session.AndroidDisplaySession
import dev.ladoflow.display.session.AndroidDisplaySessionMetrics
import dev.ladoflow.display.session.AndroidDisplaySessionState
import dev.ladoflow.display.session.DisplaySessionFailureKind
import dev.ladoflow.display.ui.theme.LadoCoral
import dev.ladoflow.display.ui.theme.LadoCyan
import dev.ladoflow.display.ui.theme.LadoFlowTheme
import dev.ladoflow.display.ui.theme.LadoMuted
import dev.ladoflow.display.ui.theme.LadoSurfaceRaised
import dev.ladoflow.display.transport.usb.AndroidUsbAccessoryTransport

private enum class Destination(
    val label: String,
    val icon: ImageVector,
) {
    Display("Display", Icons.Outlined.DisplaySettings),
    Settings("Settings", Icons.Outlined.Settings),
    Diagnostics("Diagnostics", Icons.Outlined.BugReport),
}

@Composable
fun LadoFlowApp(
    displayViewModel: DisplayViewModel = viewModel(),
    displaySession: AndroidDisplaySession? = null,
    usbTransport: AndroidUsbAccessoryTransport? = null,
    startupFailure: String? = null,
    capabilityEvidence: AndroidDisplayCapabilityEvidence? = null,
) {
    val state by displayViewModel.state.collectAsStateWithLifecycle()
    val view = LocalView.current
    val session = displaySession

    if (session != null) {
        val sessionState by session.state.collectAsStateWithLifecycle()
        val sessionMetrics by session.metrics.collectAsStateWithLifecycle()
        LaunchedEffect(sessionState) {
            displayViewModel.accept(sessionState.toDisplayEvent())
        }
        LaunchedEffect(sessionMetrics) {
            displayViewModel.accept(DisplayEvent.MetricsUpdated(sessionMetrics.toUiMetrics()))
        }
    } else if (startupFailure != null) {
        LaunchedEffect(startupFailure) {
            displayViewModel.accept(DisplayEvent.Failed(startupFailure))
        }
    }

    DisposableEffect(state.preferences.keepScreenAwake, view) {
        val previous = view.keepScreenOn
        view.keepScreenOn = state.preferences.keepScreenAwake
        onDispose { view.keepScreenOn = previous }
    }

    LadoFlowApp(
        state = state,
        surfaceController = session?.surfaceController,
        inputController = session?.inputController,
        capabilityEvidence = capabilityEvidence,
        onEvent = { event ->
            if (event == DisplayEvent.RetryRequested && startupFailure != null) {
                displayViewModel.accept(DisplayEvent.Failed(startupFailure))
            } else {
                when (event) {
                    DisplayEvent.RetryRequested -> session?.retry() ?: usbTransport?.retry()
                    DisplayEvent.Disconnected -> session?.disconnect() ?: usbTransport?.disconnect()
                    else -> Unit
                }
                displayViewModel.accept(event)
            }
        },
    )
}

private fun AndroidDisplaySessionState.toDisplayEvent(): DisplayEvent = when (this) {
    AndroidDisplaySessionState.Stopped -> DisplayEvent.TransportStopped
    AndroidDisplaySessionState.WaitingForAccessory -> DisplayEvent.RetryRequested
    is AndroidDisplaySessionState.WaitingForPermission -> DisplayEvent.AccessoryAttached(accessoryName)
    is AndroidDisplaySessionState.Handshaking -> DisplayEvent.UsbLinkConnected(accessoryName)
    is AndroidDisplaySessionState.Ready -> DisplayEvent.PairingCompleted(hostName)
    is AndroidDisplaySessionState.Configured -> DisplayEvent.StreamConfigured(
        configuration.toUiConfiguration(),
        hostName,
    )
    is AndroidDisplaySessionState.Connected -> DisplayEvent.DecoderSurfaceReady(
        configuration.toUiConfiguration(),
        hostName,
    )
    is AndroidDisplaySessionState.DeviceDisconnected ->
        DisplayEvent.DeviceDisconnected(accessoryName)
    is AndroidDisplaySessionState.Displaying -> DisplayEvent.StreamStarted(
        configuration.toUiConfiguration(),
        hostName,
    )
    is AndroidDisplaySessionState.Recovering -> DisplayEvent.RecoveryAttempted(attempt, reason)
    is AndroidDisplaySessionState.Failed -> when (kind) {
        DisplaySessionFailureKind.Protocol -> DisplayEvent.ProtocolFailed(reason)
        else -> DisplayEvent.Failed(reason)
    }
    is AndroidDisplaySessionState.Unsupported -> DisplayEvent.Failed(reason)
}

private fun DisplayConfigPayload.toUiConfiguration(): StreamConfiguration = StreamConfiguration(
    width = width,
    height = height,
    refreshRateHz = ((refreshMillihz.toULong() + 500uL) / 1_000uL).toInt(),
    codec = "H.264 Main",
)

private fun AndroidDisplaySessionMetrics.toUiMetrics(): StreamMetrics = StreamMetrics(
    releasedToSurfaceFrames = outputsReleasedToSurface,
    droppedFrames = droppedVideoFrames,
    queueDepth = queueDepth,
    decodeLatencyMillis = latestDecodeDurationMicros?.toDouble()?.div(1_000.0),
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun LadoFlowApp(
    state: DisplayUiState,
    surfaceController: DecoderSurfaceController? = null,
    inputController: AndroidInputController? = null,
    capabilityEvidence: AndroidDisplayCapabilityEvidence? = null,
    onEvent: (DisplayEvent) -> Unit,
) {
    var selectedName by rememberSaveable { mutableStateOf(Destination.Display.name) }
    val selected = Destination.valueOf(selectedName)

    LadoFlowTheme {
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val useRail = maxWidth >= 760.dp
            Scaffold(
                containerColor = MaterialTheme.colorScheme.background,
                topBar = {
                    LadoTopBar(state)
                },
                bottomBar = {
                    if (!useRail) {
                        DestinationBar(selected) { selectedName = it.name }
                    }
                },
            ) { padding ->
                Row(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(padding),
                ) {
                    if (useRail) {
                        DestinationRail(selected) { selectedName = it.name }
                    }
                    AnimatedContent(
                        targetState = selected,
                        label = "destination",
                        modifier = Modifier
                            .weight(1f)
                            .fillMaxHeight(),
                    ) { destination ->
                        when (destination) {
                            Destination.Display -> DisplayScreen(
                                state,
                                surfaceController,
                                inputController,
                                onEvent,
                            )
                            Destination.Settings -> SettingsScreen(state, onEvent)
                            Destination.Diagnostics -> DiagnosticsScreen(state, capabilityEvidence)
                        }
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun LadoTopBar(state: DisplayUiState) {
    TopAppBar(
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = MaterialTheme.colorScheme.background.copy(alpha = 0.96f),
        ),
        title = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                LadoMark()
                Spacer(Modifier.width(12.dp))
                Column {
                    Text(
                        text = "LadoFlow",
                        style = MaterialTheme.typography.titleLarge,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        text = "Local display",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        },
        actions = {
            Row(
                modifier = Modifier
                    .padding(end = 16.dp)
                    .clip(RoundedCornerShape(999.dp))
                    .background(MaterialTheme.colorScheme.surfaceVariant)
                    .padding(horizontal = 12.dp, vertical = 7.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(7.dp),
            ) {
                Box(
                    Modifier
                        .size(8.dp)
                        .background(stageColor(state.stage), CircleShape),
                )
                Text(
                    text = stageLabel(state.stage),
                    style = MaterialTheme.typography.labelMedium,
                )
            }
        },
    )
}

@Composable
private fun LadoMark() {
    Box(
        modifier = Modifier
            .size(38.dp)
            .clip(RoundedCornerShape(11.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier
                .width(22.dp)
                .height(15.dp)
                .border(2.dp, LadoCyan, RoundedCornerShape(4.dp)),
        )
        Box(
            Modifier
                .padding(start = 15.dp, top = 11.dp)
                .size(width = 13.dp, height = 17.dp)
                .border(2.dp, LadoCyan, RoundedCornerShape(4.dp)),
        )
        Box(
            Modifier
                .padding(start = 23.dp, top = 17.dp)
                .size(4.dp)
                .background(LadoCoral, CircleShape),
        )
    }
}

@Composable
private fun DestinationBar(
    selected: Destination,
    onSelected: (Destination) -> Unit,
) {
    NavigationBar(containerColor = MaterialTheme.colorScheme.surface) {
        Destination.entries.forEach { destination ->
            NavigationBarItem(
                selected = selected == destination,
                onClick = { onSelected(destination) },
                icon = { Icon(destination.icon, contentDescription = null) },
                label = { Text(destination.label) },
            )
        }
    }
}

@Composable
private fun DestinationRail(
    selected: Destination,
    onSelected: (Destination) -> Unit,
) {
    NavigationRail(containerColor = MaterialTheme.colorScheme.surface) {
        Spacer(Modifier.height(12.dp))
        Destination.entries.forEach { destination ->
            NavigationRailItem(
                selected = selected == destination,
                onClick = { onSelected(destination) },
                icon = { Icon(destination.icon, contentDescription = null) },
                label = { Text(destination.label) },
            )
        }
    }
}

@Composable
private fun DisplayScreen(
    state: DisplayUiState,
    surfaceController: DecoderSurfaceController?,
    inputController: AndroidInputController?,
    onEvent: (DisplayEvent) -> Unit,
) {
    Box(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 20.dp, vertical = 18.dp),
        contentAlignment = Alignment.TopCenter,
    ) {
        Column(
            modifier = Modifier.widthIn(max = 1040.dp),
            verticalArrangement = Arrangement.spacedBy(18.dp),
        ) {
            if (state.stream != null && surfaceController != null) {
                DisplaySurfaceCard(state, surfaceController, inputController)
                ActiveSessionControls(state, onEvent)
            } else {
                ConnectionHero(state, onEvent)
            }
            ConnectionJourney(state.stage)
            PrivacyCard()
        }
    }
}

@Composable
private fun ActiveSessionControls(
    state: DisplayUiState,
    onEvent: (DisplayEvent) -> Unit,
) {
    Card(
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        shape = RoundedCornerShape(18.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    state.hostName ?: "LadoFlow Host",
                    style = MaterialTheme.typography.titleSmall,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    state.detail,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            OutlinedButton(onClick = { onEvent(DisplayEvent.Disconnected) }) {
                Text("Disconnect")
            }
        }
    }
}

@Composable
private fun ConnectionHero(
    state: DisplayUiState,
    onEvent: (DisplayEvent) -> Unit,
) {
    Card(
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        shape = RoundedCornerShape(28.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 28.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            StageOrb(state.stage)
            Spacer(Modifier.height(22.dp))
            Text(
                text = stageHeadline(state.stage),
                style = MaterialTheme.typography.headlineSmall,
                fontWeight = FontWeight.SemiBold,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(9.dp))
            Text(
                text = state.detail,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.widthIn(max = 560.dp),
            )

            state.hostName?.let { host ->
                Spacer(Modifier.height(18.dp))
                InfoPill("Host", host)
            }
            state.lastError?.let { error ->
                Spacer(Modifier.height(16.dp))
                Text(
                    text = error,
                    color = MaterialTheme.colorScheme.error,
                    style = MaterialTheme.typography.bodyMedium,
                    textAlign = TextAlign.Center,
                )
            }

            when (state.stage) {
                ConnectionStage.Disconnected,
                ConnectionStage.DeviceDisconnected,
                ConnectionStage.WaitingForAccessory,
                ConnectionStage.ProtocolError,
                ConnectionStage.Error,
                -> {
                    Spacer(Modifier.height(22.dp))
                    Button(onClick = { onEvent(DisplayEvent.RetryRequested) }) {
                        Icon(Icons.Outlined.Usb, contentDescription = null)
                        Spacer(Modifier.width(8.dp))
                        Text("Check USB connection")
                    }
                }

                ConnectionStage.Connected -> {
                    Spacer(Modifier.height(22.dp))
                    OutlinedButton(onClick = { onEvent(DisplayEvent.Disconnected) }) {
                        Text("Disconnect")
                    }
                }

                else -> Unit
            }
        }
    }
}

@Composable
private fun StageOrb(stage: ConnectionStage) {
    val transition = rememberInfiniteTransition(label = "connection pulse")
    val pulse by transition.animateFloat(
        initialValue = 0.92f,
        targetValue = 1.06f,
        animationSpec = infiniteRepeatable(
            animation = tween(1_100),
            repeatMode = RepeatMode.Reverse,
        ),
        label = "connection pulse scale",
    )
    val active = stage == ConnectionStage.Pairing ||
        stage == ConnectionStage.Recovering ||
        stage == ConnectionStage.WaitingForPermission

    Box(
        modifier = Modifier
            .size(96.dp)
            .scale(if (active) pulse else 1f)
            .clip(CircleShape)
            .background(stageColor(stage).copy(alpha = 0.12f)),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            imageVector = when (stage) {
                ConnectionStage.WaitingForAccessory,
                ConnectionStage.WaitingForPermission,
                -> Icons.Outlined.Cable

                ConnectionStage.DeviceDisconnected,
                ConnectionStage.ProtocolError,
                ConnectionStage.Error,
                ConnectionStage.Disconnected,
                -> Icons.Outlined.Usb

                else -> Icons.Outlined.DisplaySettings
            },
            contentDescription = null,
            tint = stageColor(stage),
            modifier = Modifier.size(42.dp),
        )
    }
}

@Composable
private fun DisplaySurfaceCard(
    state: DisplayUiState,
    surfaceController: DecoderSurfaceController,
    inputController: AndroidInputController?,
) {
    val stream = requireNotNull(state.stream)
    Card(
        colors = CardDefaults.cardColors(containerColor = Color.Black),
        shape = RoundedCornerShape(24.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .aspectRatio(stream.width.toFloat() / stream.height)
                .background(Color.Black),
            contentAlignment = Alignment.Center,
        ) {
            MediaCodecSurface(
                surfaceController = surfaceController,
                inputController = inputController,
                modifier = Modifier.fillMaxSize(),
            )

            if (state.stage != ConnectionStage.Displaying) {
                Text(
                    text = when (state.stage) {
                        ConnectionStage.Recovering -> "Recovering decoder session"
                        else -> "Surface ready · waiting for H.264 output"
                    },
                    style = MaterialTheme.typography.titleMedium,
                    color = Color.White,
                    modifier = Modifier
                        .clip(RoundedCornerShape(12.dp))
                        .background(Color.Black.copy(alpha = 0.72f))
                        .padding(horizontal = 14.dp, vertical = 10.dp),
                )
            }

            Text(
                text = "${stream.width} × ${stream.height} · ${stream.refreshRateHz} Hz · ${stream.codec}",
                style = MaterialTheme.typography.bodySmall,
                color = Color.White.copy(alpha = 0.82f),
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .padding(12.dp)
                    .clip(RoundedCornerShape(10.dp))
                    .background(Color.Black.copy(alpha = 0.72f))
                    .padding(horizontal = 10.dp, vertical = 7.dp),
            )

            if (state.preferences.showDiagnosticsOverlay) {
                Column(
                    modifier = Modifier
                        .align(Alignment.TopEnd)
                        .padding(12.dp)
                        .clip(RoundedCornerShape(10.dp))
                        .background(Color.Black.copy(alpha = 0.72f))
                        .padding(10.dp),
                ) {
                    Text("Queue ${state.metrics.queueDepth}", color = Color.White)
                    Text("Dropped ${state.metrics.droppedFrames}", color = Color.White)
                    Text(
                        state.metrics.decodeLatencyMillis?.let { "Decode %.1f ms".format(it) }
                            ?: "Decode —",
                        color = Color.White,
                    )
                }
            }
        }
    }
}

@Composable
private fun ConnectionJourney(stage: ConnectionStage) {
    Card(
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        shape = RoundedCornerShape(22.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(Modifier.padding(20.dp)) {
            Text(
                text = "USB-first connection",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
            )
            Spacer(Modifier.height(6.dp))
            Text(
                text = "No developer mode or ADB is required.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(18.dp))
            JourneyStep(
                index = 1,
                title = "Connect the cable",
                detail = "Use a data-capable USB cable between this device and the host.",
                complete = stage.hasAccessoryConnection(),
            )
            JourneyStep(
                index = 2,
                title = "Approve LadoFlow",
                detail = "Android asks before opening the USB accessory.",
                complete = stage.hasUsbAuthorization(),
            )
            JourneyStep(
                index = 3,
                title = "Start the extended display",
                detail = "The host negotiates a local, hardware-decoded stream.",
                complete = stage == ConnectionStage.Displaying,
                showConnector = false,
            )
        }
    }
}

@Composable
private fun JourneyStep(
    index: Int,
    title: String,
    detail: String,
    complete: Boolean,
    showConnector: Boolean = true,
) {
    Row {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Box(
                modifier = Modifier
                    .size(30.dp)
                    .background(
                        if (complete) LadoCyan else MaterialTheme.colorScheme.surfaceVariant,
                        CircleShape,
                    ),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = if (complete) "✓" else index.toString(),
                    color = if (complete) Color(0xFF00201D) else MaterialTheme.colorScheme.onSurfaceVariant,
                    fontWeight = FontWeight.Bold,
                    style = MaterialTheme.typography.labelMedium,
                )
            }
            if (showConnector) {
                Box(
                    Modifier
                        .width(2.dp)
                        .height(40.dp)
                        .background(MaterialTheme.colorScheme.surfaceVariant),
                )
            }
        }
        Spacer(Modifier.width(14.dp))
        Column(Modifier.padding(top = 3.dp)) {
            Text(title, fontWeight = FontWeight.Medium)
            Text(
                detail,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun PrivacyCard() {
    Card(
        colors = CardDefaults.cardColors(containerColor = LadoSurfaceRaised),
        shape = RoundedCornerShape(20.dp),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(18.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Outlined.Lock, contentDescription = null, tint = LadoCyan)
            Spacer(Modifier.width(14.dp))
            Column {
                Text("Local by default", fontWeight = FontWeight.SemiBold)
                Text(
                    "No account, cloud relay, or analytics are required for the core display path.",
                    style = MaterialTheme.typography.bodySmall,
                    color = LadoMuted,
                )
            }
        }
    }
}

@Composable
private fun SettingsScreen(
    state: DisplayUiState,
    onEvent: (DisplayEvent) -> Unit,
) {
    ScreenColumn(title = "Display settings", subtitle = "Preferences remain on this device.") {
        SettingsCard(title = "Quality mode") {
            Text(
                "Automatic adapts to the negotiated USB link and decoder limits.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(14.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                QualityMode.entries.forEach { mode ->
                    FilterChip(
                        selected = state.preferences.qualityMode == mode,
                        onClick = {
                            onEvent(
                                DisplayEvent.PreferencesChanged(
                                    state.preferences.copy(qualityMode = mode),
                                ),
                            )
                        },
                        label = { Text(mode.label()) },
                    )
                }
            }
        }
        SettingsCard(title = "Screen") {
            PreferenceToggle(
                title = "Keep screen awake",
                detail = "Prevent sleep while LadoFlow is open.",
                checked = state.preferences.keepScreenAwake,
                onChecked = { value ->
                    onEvent(
                        DisplayEvent.PreferencesChanged(
                            state.preferences.copy(keepScreenAwake = value),
                        ),
                    )
                },
            )
            HorizontalDivider(Modifier.padding(vertical = 10.dp))
            PreferenceToggle(
                title = "Diagnostics overlay",
                detail = "Show queue, drop, and decode timing while displaying.",
                checked = state.preferences.showDiagnosticsOverlay,
                onChecked = { value ->
                    onEvent(
                        DisplayEvent.PreferencesChanged(
                            state.preferences.copy(showDiagnosticsOverlay = value),
                        ),
                    )
                },
            )
        }
    }
}

@Composable
private fun DiagnosticsScreen(
    state: DisplayUiState,
    capabilityEvidence: AndroidDisplayCapabilityEvidence?,
) {
    ScreenColumn(
        title = "Diagnostics",
        subtitle = "Honest implementation status for this Android build.",
    ) {
        SettingsCard(title = "Connection") {
            DiagnosticRow("State", stageLabel(state.stage))
            DiagnosticRow(
                "Transport",
                when (state.stage) {
                    ConnectionStage.Pairing,
                    ConnectionStage.Connected,
                    ConnectionStage.Displaying,
                    -> "USB Accessory · link open"

                    ConnectionStage.Recovering -> "USB Accessory · recovering"
                    else -> "USB Accessory · not connected"
                },
            )
            DiagnosticRow("Host", state.hostName ?: "Not connected")
            DiagnosticRow("Protocol", "LDFL v1 · frame and payload codec ready")
            DiagnosticRow("Reverse input", "Pointer · touch · keyboard")
        }
        SettingsCard(title = "Decoder") {
            DiagnosticRow("Negotiated codec", "H.264 Main")
            DiagnosticRow(
                "Capability decoder",
                capabilityEvidence?.decoderName ?: "Not available",
            )
            DiagnosticRow(
                "Acceleration evidence",
                capabilityEvidence?.hardwareAcceleration?.name ?: "Not queried",
            )
            DiagnosticRow("Released to Surface", state.metrics.releasedToSurfaceFrames.toString())
            DiagnosticRow("Dropped frames", state.metrics.droppedFrames.toString())
            DiagnosticRow("Queue depth", state.metrics.queueDepth.toString())
        }
        SettingsCard(title = "Build boundary") {
            Text(
                "This build includes the Compose UI, bounded LDFL v1 codec, and Android USB Accessory " +
                    "lifecycle/I/O boundary. The H.264 MediaCodec boundary validates Annex-B and waits " +
                    "for SPS/PPS plus an LDFL keyframe. 未实机验证: USB and MediaCodec output have " +
                    "not been verified on a physical Android device.",
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                style = MaterialTheme.typography.bodyMedium,
            )
        }
    }
}

@Composable
private fun ScreenColumn(
    title: String,
    subtitle: String,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 20.dp, vertical = 18.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Column(Modifier.widthIn(max = 900.dp).fillMaxWidth()) {
            Text(title, style = MaterialTheme.typography.headlineMedium, fontWeight = FontWeight.SemiBold)
            Text(
                subtitle,
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(20.dp))
            content()
        }
    }
}

@Composable
private fun SettingsCard(
    title: String,
    content: @Composable ColumnScope.() -> Unit,
) {
    Card(
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surface),
        shape = RoundedCornerShape(20.dp),
        modifier = Modifier
            .fillMaxWidth()
            .padding(bottom = 14.dp),
    ) {
        Column(Modifier.padding(20.dp)) {
            Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Spacer(Modifier.height(14.dp))
            content()
        }
    }
}

@Composable
private fun PreferenceToggle(
    title: String,
    detail: String,
    checked: Boolean,
    onChecked: (Boolean) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(title, fontWeight = FontWeight.Medium)
            Text(
                detail,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Spacer(Modifier.width(16.dp))
        Switch(checked = checked, onCheckedChange = onChecked)
    }
}

@Composable
private fun DiagnosticRow(label: String, value: String) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 7.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(label, color = MaterialTheme.colorScheme.onSurfaceVariant)
        Spacer(Modifier.width(18.dp))
        Text(value, fontWeight = FontWeight.Medium, textAlign = TextAlign.End)
    }
}

@Composable
private fun InfoPill(label: String, value: String) {
    Row(
        modifier = Modifier
            .clip(RoundedCornerShape(999.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .padding(horizontal = 14.dp, vertical = 9.dp),
    ) {
        Text("$label · ", color = MaterialTheme.colorScheme.onSurfaceVariant)
        Text(value, fontWeight = FontWeight.SemiBold)
    }
}

private fun QualityMode.label(): String = when (this) {
    QualityMode.Automatic -> "Auto"
    QualityMode.Quality -> "Quality"
    QualityMode.Balanced -> "Balanced"
    QualityMode.LowLatency -> "Low latency"
}

private fun stageLabel(stage: ConnectionStage): String = when (stage) {
    ConnectionStage.Disconnected -> "Disconnected"
    ConnectionStage.DeviceDisconnected -> "Device disconnected"
    ConnectionStage.WaitingForAccessory -> "USB ready"
    ConnectionStage.WaitingForPermission -> "Waiting for authorization"
    ConnectionStage.Pairing -> "Pairing"
    ConnectionStage.Connected -> "Connected"
    ConnectionStage.Displaying -> "Displaying"
    ConnectionStage.Recovering -> "Reconnecting"
    ConnectionStage.ProtocolError -> "Protocol error"
    ConnectionStage.Error -> "Needs attention"
}

private fun stageHeadline(stage: ConnectionStage): String = when (stage) {
    ConnectionStage.Disconnected -> "Host disconnected"
    ConnectionStage.DeviceDisconnected -> "Device disconnected"
    ConnectionStage.WaitingForAccessory -> "Connect your computer"
    ConnectionStage.WaitingForPermission -> "Waiting for authorization"
    ConnectionStage.Pairing -> "Pairing with your host"
    ConnectionStage.Connected -> "Connected"
    ConnectionStage.Displaying -> "Extended display active"
    ConnectionStage.Recovering -> "Reconnecting"
    ConnectionStage.ProtocolError -> "Protocol error"
    ConnectionStage.Error -> "Connection needs attention"
}

private fun stageColor(stage: ConnectionStage): Color = when (stage) {
    ConnectionStage.Connected,
    ConnectionStage.Displaying,
    -> LadoCyan

    ConnectionStage.Pairing,
    ConnectionStage.Recovering,
    ConnectionStage.WaitingForPermission,
    -> Color(0xFFFFC857)

    ConnectionStage.ProtocolError,
    ConnectionStage.Error,
    -> Color(0xFFFF8A80)

    ConnectionStage.DeviceDisconnected,
    ConnectionStage.Disconnected,
    -> Color(0xFF8FA3B2)

    ConnectionStage.WaitingForAccessory -> Color(0xFF78A9FF)
}

private fun ConnectionStage.hasAccessoryConnection(): Boolean = when (this) {
    ConnectionStage.WaitingForPermission,
    ConnectionStage.Pairing,
    ConnectionStage.Connected,
    ConnectionStage.Displaying,
    ConnectionStage.Recovering,
    ConnectionStage.ProtocolError,
    ConnectionStage.Error,
    -> true

    ConnectionStage.Disconnected,
    ConnectionStage.DeviceDisconnected,
    ConnectionStage.WaitingForAccessory,
    -> false
}

private fun ConnectionStage.hasUsbAuthorization(): Boolean = when (this) {
    ConnectionStage.Pairing,
    ConnectionStage.Connected,
    ConnectionStage.Displaying,
    ConnectionStage.Recovering,
    ConnectionStage.ProtocolError,
    -> true

    ConnectionStage.Disconnected,
    ConnectionStage.DeviceDisconnected,
    ConnectionStage.WaitingForAccessory,
    ConnectionStage.WaitingForPermission,
    ConnectionStage.Error,
    -> false
}

@Preview(showBackground = true, backgroundColor = 0xFF07131F, widthDp = 420, heightDp = 900)
@Composable
private fun WaitingPreview() {
    LadoFlowApp(state = DisplayUiState(), onEvent = {})
}

@Preview(showBackground = true, backgroundColor = 0xFF07131F, widthDp = 900, heightDp = 650)
@Composable
private fun DisplayingPreview() {
    LadoFlowApp(
        state = DisplayUiState(
            stage = ConnectionStage.Displaying,
            hostName = "Studio PC",
            detail = "The host is sending this extended display.",
            stream = StreamConfiguration(1920, 1080, 60, "H.264 Main"),
        ),
        onEvent = {},
    )
}
