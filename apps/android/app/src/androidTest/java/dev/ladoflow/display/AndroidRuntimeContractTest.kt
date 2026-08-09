package dev.ladoflow.display

import android.content.ComponentName
import android.content.pm.PackageManager
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import dev.ladoflow.display.media.probeAndroidDisplayCapabilities
import dev.ladoflow.display.protocol.CodecCapabilities
import dev.ladoflow.display.protocol.InputCapabilities
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class AndroidRuntimeContractTest {
    @Suppress("DEPRECATION")
    @Test
    fun mainActivityPublishesTheAccessoryAttachMetadata() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val activity = context.packageManager.getActivityInfo(
            ComponentName(context, MainActivity::class.java),
            PackageManager.GET_META_DATA,
        )

        assertNotNull(activity.metaData)
        assertTrue(
            activity.metaData.containsKey("android.hardware.usb.action.USB_ACCESSORY_ATTACHED"),
        )
    }

    @Test
    fun capabilityProbeReturnsEvidenceOrAnActionableUnsupportedReason() {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        val result = runCatching { probeAndroidDisplayCapabilities(context) }

        result.onSuccess { evidence ->
            assertTrue(evidence.decoderName.isNotBlank())
            assertTrue(evidence.capabilities.codecs.contains(CodecCapabilities.H264))
            assertTrue(evidence.capabilities.input.contains(InputCapabilities.Keyboard))
            assertTrue(evidence.capabilities.maxRefreshMillihz >= 30_000u)
        }.onFailure { failure ->
            assertTrue(failure.message?.isNotBlank() == true)
        }
    }
}
