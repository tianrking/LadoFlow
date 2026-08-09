package dev.ladoflow.display.transport.usb

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.hardware.usb.UsbAccessory
import android.hardware.usb.UsbManager
import android.os.Build
import androidx.core.content.ContextCompat
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import dev.ladoflow.display.BuildConfig
import dev.ladoflow.display.protocol.LdflFrame
import java.io.Closeable
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

class AndroidUsbAccessoryTransport(
    context: Context,
    private val reconnectPolicy: UsbReconnectPolicy = UsbReconnectPolicy(),
) : DefaultLifecycleObserver, Closeable, LdflDisplayTransport {
    private val applicationContext = context.applicationContext
    private val usbManager = applicationContext.getSystemService(UsbManager::class.java)
    private val usbAccessorySupported = applicationContext.packageManager.hasSystemFeature(
        PackageManager.FEATURE_USB_ACCESSORY,
    )
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val mutableState = MutableStateFlow<UsbTransportState>(UsbTransportState.Stopped)
    private val mutableSession = MutableStateFlow<UsbIoSession?>(null)
    private val started = AtomicBoolean(false)
    private val closed = AtomicBoolean(false)

    private var foreground = false
    private var userPaused = false
    private var permissionRequestInFlight = false
    private var currentAccessory: UsbAccessory? = null
    private var sessionStateJob: Job? = null
    private var openJob: Job? = null
    private var reconnectJob: Job? = null
    private var reconnectAttempt = 0

    override val state: StateFlow<UsbTransportState> = mutableState.asStateFlow()

    @OptIn(ExperimentalCoroutinesApi::class)
    override val frames: Flow<LdflFrame> = mutableSession
        .filterNotNull()
        .flatMapLatest { it.frames }

    private val permissionReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            if (intent?.action != ACTION_USB_PERMISSION) return
            permissionRequestInFlight = false
            val accessory = intent.usbAccessory() ?: currentAccessory ?: return
            if (!accessory.identity().isLadoFlowHost) return
            currentAccessory = accessory
            if (intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)) {
                maybeOpenAccessory(accessory)
            } else {
                mutableState.value = UsbTransportState.Error(
                    accessory = accessory.identity(),
                    reason = "Android USB accessory permission was denied",
                    retryable = true,
                )
            }
        }
    }

    private val detachReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            if (intent?.action != UsbManager.ACTION_USB_ACCESSORY_DETACHED) return
            val detached = intent.usbAccessory() ?: return
            if (detached != currentAccessory) return
            if (isAttached(detached)) return
            permissionRequestInFlight = false
            currentAccessory = null
            reconnectAttempt = 0
            cancelConnectionWork()
            closeActiveSession()
            mutableState.value = if (foreground && !userPaused) {
                UsbTransportState.WaitingForAccessory
            } else {
                UsbTransportState.Stopped
            }
        }
    }

    fun start() {
        if (!started.compareAndSet(false, true)) return
        if (!usbAccessorySupported) {
            mutableState.value = UsbTransportState.Unsupported(
                "This Android device does not report USB Accessory support",
            )
            return
        }
        ContextCompat.registerReceiver(
            applicationContext,
            permissionReceiver,
            IntentFilter(ACTION_USB_PERMISSION),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
        ContextCompat.registerReceiver(
            applicationContext,
            detachReceiver,
            IntentFilter(UsbManager.ACTION_USB_ACCESSORY_DETACHED),
            ContextCompat.RECEIVER_EXPORTED,
        )
    }

    override fun onStart(owner: LifecycleOwner) {
        foreground = true
        if (!usbAccessorySupported) {
            mutableState.value = UsbTransportState.Unsupported(
                "This Android device does not report USB Accessory support",
            )
            return
        }
        if (!userPaused) scanAttachedAccessories()
    }

    override fun onStop(owner: LifecycleOwner) {
        foreground = false
        cancelConnectionWork()
        closeActiveSession()
        mutableState.value = UsbTransportState.Stopped
    }

    fun handleIntent(intent: Intent?) {
        if (intent?.action != UsbManager.ACTION_USB_ACCESSORY_ATTACHED) return
        val accessory = intent.usbAccessory() ?: return
        handleAttachedAccessory(accessory)
    }

    override fun retry() {
        userPaused = false
        reconnectAttempt = 0
        if (!usbAccessorySupported) {
            mutableState.value = UsbTransportState.Unsupported(
                "This Android device does not report USB Accessory support",
            )
            return
        }
        if (foreground) scanAttachedAccessories()
    }

    override fun disconnect() {
        userPaused = true
        cancelConnectionWork()
        closeActiveSession()
        mutableState.value = UsbTransportState.Stopped
    }

    override suspend fun sendControl(frame: LdflFrame): Boolean {
        val session = mutableSession.value ?: return false
        session.sendControl(frame)
        return true
    }

    override fun trySendControl(frame: LdflFrame): Boolean =
        mutableSession.value?.trySendControl(frame) == true

    private fun scanAttachedAccessories() {
        if (closed.get() || userPaused || !foreground || !usbAccessorySupported) return
        val accessory = usbManager.accessoryList
            ?.firstOrNull { it.identity().isLadoFlowHost }
        if (accessory == null) {
            currentAccessory = null
            closeActiveSession()
            mutableState.value = UsbTransportState.WaitingForAccessory
        } else {
            handleAttachedAccessory(accessory)
        }
    }

    private fun handleAttachedAccessory(accessory: UsbAccessory) {
        val identity = accessory.identity()
        if (!identity.isLadoFlowHost) return
        currentAccessory = accessory
        if (!foreground || userPaused) {
            mutableState.value = UsbTransportState.Stopped
            return
        }
        if (usbManager.hasPermission(accessory)) {
            maybeOpenAccessory(accessory)
        } else {
            requestPermission(accessory)
        }
    }

    private fun requestPermission(accessory: UsbAccessory) {
        if (permissionRequestInFlight) return
        permissionRequestInFlight = true
        mutableState.value = UsbTransportState.AwaitingPermission(accessory.identity())
        val flags = PendingIntent.FLAG_UPDATE_CURRENT or
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0
        val permissionIntent = PendingIntent.getBroadcast(
            applicationContext,
            USB_PERMISSION_REQUEST_CODE,
            Intent(ACTION_USB_PERMISSION).setPackage(applicationContext.packageName),
            flags,
        )
        runCatching { usbManager.requestPermission(accessory, permissionIntent) }
            .onFailure { exception ->
                permissionRequestInFlight = false
                mutableState.value = UsbTransportState.Error(
                    accessory = accessory.identity(),
                    reason = exception.message ?: "Unable to request Android USB permission",
                    retryable = true,
                )
            }
    }

    private fun maybeOpenAccessory(accessory: UsbAccessory) {
        if (!foreground || userPaused || !isAttached(accessory)) return
        if (mutableSession.value != null && currentAccessory == accessory) return
        openJob?.cancel()
        openJob = scope.launch {
            val openedConnection = AtomicReference<AndroidUsbDuplexConnection?>(null)
            try {
                mutableState.value = UsbTransportState.Opening(accessory.identity())
                val connection = try {
                    withContext(Dispatchers.IO) {
                        usbManager.openAccessory(accessory)
                            ?.let(AndroidUsbDuplexConnection::from)
                            ?.also(openedConnection::set)
                    }
                } catch (cancelled: CancellationException) {
                    throw cancelled
                } catch (exception: Exception) {
                    scheduleReconnect(
                        accessory,
                        exception.message ?: "UsbManager.openAccessory failed",
                    )
                    return@launch
                }
                if (connection == null) {
                    scheduleReconnect(
                        accessory,
                        "UsbManager.openAccessory returned no descriptor",
                    )
                    return@launch
                }
                if (!foreground || userPaused || accessory != currentAccessory || !isAttached(accessory)) {
                    return@launch
                }

                closeActiveSession()
                val session = UsbIoSession(connection, scope)
                mutableSession.value = session
                session.start()
                reconnectAttempt = 0
                mutableState.value = UsbTransportState.Connected(accessory.identity())
                sessionStateJob = scope.launch {
                    session.state.collect { sessionState ->
                        if (sessionState is UsbIoSessionState.Failed && mutableSession.value === session) {
                            mutableSession.value = null
                            scheduleReconnect(accessory, sessionState.message)
                        }
                    }
                }
                openedConnection.set(null)
            } finally {
                openedConnection.getAndSet(null)?.close()
            }
        }
    }

    private fun scheduleReconnect(accessory: UsbAccessory, reason: String) {
        closeActiveSession()
        if (!foreground || userPaused || !isAttached(accessory)) {
            mutableState.value = if (foreground && !userPaused) {
                UsbTransportState.WaitingForAccessory
            } else {
                UsbTransportState.Stopped
            }
            return
        }
        val attempt = reconnectAttempt + 1
        reconnectAttempt = attempt
        if (!reconnectPolicy.shouldRetry(attempt)) {
            mutableState.value = UsbTransportState.Error(
                accessory = accessory.identity(),
                reason = reason,
                retryable = true,
            )
            return
        }
        val delayMillis = reconnectPolicy.delayMillis(attempt)
        mutableState.value = UsbTransportState.Recovering(
            accessory = accessory.identity(),
            attempt = attempt,
            delayMillis = delayMillis,
            reason = reason,
        )
        reconnectJob?.cancel()
        reconnectJob = scope.launch {
            delay(delayMillis)
            if (usbManager.hasPermission(accessory)) {
                maybeOpenAccessory(accessory)
            } else {
                requestPermission(accessory)
            }
        }
    }

    private fun isAttached(accessory: UsbAccessory): Boolean =
        usbManager.accessoryList?.any { it == accessory } == true

    private fun cancelConnectionWork() {
        openJob?.cancel()
        openJob = null
        reconnectJob?.cancel()
        reconnectJob = null
    }

    private fun closeActiveSession() {
        sessionStateJob?.cancel()
        sessionStateJob = null
        val session = mutableSession.value
        mutableSession.value = null
        session?.close()
    }

    override fun close() {
        if (!closed.compareAndSet(false, true)) return
        cancelConnectionWork()
        closeActiveSession()
        if (started.get()) {
            runCatching { applicationContext.unregisterReceiver(permissionReceiver) }
            runCatching { applicationContext.unregisterReceiver(detachReceiver) }
        }
        scope.coroutineContext[Job]?.cancel()
        mutableState.value = UsbTransportState.Stopped
    }

    private fun UsbAccessory.identity(): UsbAccessoryIdentity = UsbAccessoryIdentity(
        manufacturer = manufacturer,
        model = model,
        description = description,
        version = version,
        serial = serial,
    )

    @Suppress("DEPRECATION")
    private fun Intent.usbAccessory(): UsbAccessory? = if (Build.VERSION.SDK_INT >= 33) {
        getParcelableExtra(UsbManager.EXTRA_ACCESSORY, UsbAccessory::class.java)
    } else {
        getParcelableExtra(UsbManager.EXTRA_ACCESSORY)
    }

    companion object {
        private const val USB_PERMISSION_REQUEST_CODE = 31_041
        private val ACTION_USB_PERMISSION = "${BuildConfig.APPLICATION_ID}.USB_ACCESSORY_PERMISSION"
    }
}
