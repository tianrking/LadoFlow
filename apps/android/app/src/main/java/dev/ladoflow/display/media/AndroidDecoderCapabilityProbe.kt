package dev.ladoflow.display.media

import android.content.Context
import android.hardware.display.DisplayManager
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.os.Build
import android.view.Display
import dev.ladoflow.display.protocol.CapabilitiesPayload
import dev.ladoflow.display.protocol.CodecCapabilities
import dev.ladoflow.display.protocol.FeatureFlags
import dev.ladoflow.display.protocol.InputCapabilities
import kotlin.math.floor

data class AndroidDisplayCapabilityEvidence(
    val capabilities: CapabilitiesPayload,
    val decoderName: String,
    val hardwareAcceleration: HardwareAccelerationEvidence,
    val probeWidth: Int,
    val probeHeight: Int,
)

internal val androidDisplayInputCapabilities: InputCapabilities =
    InputCapabilities.Pointer or InputCapabilities.Touch or InputCapabilities.Keyboard

internal val androidDisplayFeatureCapabilities: FeatureFlags = FeatureFlags.None

internal data class CoordinatedDisplayMode(
    val width: Int,
    val height: Int,
)

/** Ordered Windows Host fallback contract; this is policy, not a new LDFL field. */
internal val coordinatedHostDisplayModes: List<CoordinatedDisplayMode> = listOf(
    CoordinatedDisplayMode(2732, 2048),
    CoordinatedDisplayMode(2560, 1600),
    CoordinatedDisplayMode(2560, 1440),
    CoordinatedDisplayMode(2048, 1536),
    CoordinatedDisplayMode(1920, 1200),
    CoordinatedDisplayMode(1920, 1080),
    CoordinatedDisplayMode(1600, 1200),
    CoordinatedDisplayMode(1366, 768),
    CoordinatedDisplayMode(1280, 800),
    CoordinatedDisplayMode(1280, 720),
    CoordinatedDisplayMode(1024, 768),
    CoordinatedDisplayMode(1024, 640),
    CoordinatedDisplayMode(960, 600),
    CoordinatedDisplayMode(960, 540),
    CoordinatedDisplayMode(800, 600),
    CoordinatedDisplayMode(800, 500),
    CoordinatedDisplayMode(640, 480),
    CoordinatedDisplayMode(640, 400),
)

/** Queries the actual display modes and H.264 Main decoder limits used for negotiation. */
fun probeAndroidDisplayCapabilities(context: Context): AndroidDisplayCapabilityEvidence {
    val displayLimits = queryDisplayLimits(context)
    val candidates = MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
        .asSequence()
        .filterNot { it.isEncoder }
        .filter { info ->
            info.supportedTypes.any { it.equals(MediaFormat.MIMETYPE_VIDEO_AVC, ignoreCase = true) }
        }
        .mapNotNull { info -> probeDecoder(info, displayLimits) }
        .filter { it.maximumRefreshMillihz >= MINIMUM_REFRESH_MILLIHZ }
        .sortedWith(
            compareBy<DecoderProbe> { it.hardwareAcceleration.selectionRank() }
                .thenByDescending { it.probeWidth.toLong() * it.probeHeight }
                .thenByDescending { it.maximumRefreshMillihz }
                .thenBy { it.decoderName },
        )
        .toList()

    val selected = candidates.firstOrNull()
        ?: throw UnsupportedOperationException(
            "No Android H.264 Main decoder can sustain at least 30 Hz at a display-sized mode",
        )
    return AndroidDisplayCapabilityEvidence(
        capabilities = CapabilitiesPayload(
            maxWidth = selected.maximumWidth,
            maxHeight = selected.maximumHeight,
            maxRefreshMillihz = selected.maximumRefreshMillihz,
            maxBitrateKbps = selected.maximumBitrateKbps,
            codecs = CodecCapabilities.H264,
            input = androidDisplayInputCapabilities,
            features = androidDisplayFeatureCapabilities,
        ),
        decoderName = selected.decoderName,
        hardwareAcceleration = selected.hardwareAcceleration,
        probeWidth = selected.probeWidth,
        probeHeight = selected.probeHeight,
    )
}

fun defaultAndroidCapabilities(context: Context): CapabilitiesPayload =
    probeAndroidDisplayCapabilities(context).capabilities

private fun probeDecoder(
    info: MediaCodecInfo,
    display: DisplayLimits,
): DecoderProbe? {
    val capabilities = runCatching {
        info.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC)
    }.getOrNull() ?: return null
    if (capabilities.profileLevels.none {
            it.profile == MediaCodecInfo.CodecProfileLevel.AVCProfileMain
        }
    ) {
        return null
    }
    val video = capabilities.videoCapabilities ?: return null
    val supportedModes = findAdvertisableStandardModes(video, display) ?: return null
    val refreshMillihz = floor(
        supportedModes.maximumRefreshHertz * 1_000.0,
    ).toLong().coerceIn(1L, UInt.MAX_VALUE.toLong()).toUInt()
    val maximumBitrateKbps = (video.bitrateRange.upper.toLong() / 1_000L)
        .coerceIn(1L, UInt.MAX_VALUE.toLong())
        .toUInt()
    val evidence = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        when {
            info.isHardwareAccelerated -> HardwareAccelerationEvidence.ReportedHardware
            info.isSoftwareOnly -> HardwareAccelerationEvidence.ReportedSoftware
            else -> HardwareAccelerationEvidence.NotReportedByPlatform
        }
    } else {
        HardwareAccelerationEvidence.NotReportedByPlatform
    }
    return DecoderProbe(
        decoderName = info.name,
        hardwareAcceleration = evidence,
        maximumWidth = supportedModes.maximumWidth,
        maximumHeight = supportedModes.maximumHeight,
        maximumRefreshMillihz = refreshMillihz,
        maximumBitrateKbps = maximumBitrateKbps,
        probeWidth = supportedModes.maximumWidth,
        probeHeight = supportedModes.maximumHeight,
    )
}

private fun findAdvertisableStandardModes(
    video: MediaCodecInfo.VideoCapabilities,
    display: DisplayLimits,
): AdvertisableStandardModes? {
    val physicalWidth = maxOf(display.width, display.height)
    val physicalHeight = minOf(display.width, display.height)
    val candidateCaps = coordinatedHostDisplayModes.filter { mode ->
        mode.width <= physicalWidth &&
            mode.height <= physicalHeight &&
            mode.width <= video.supportedWidths.upper &&
            mode.height <= video.supportedHeights.upper
    }

    for (candidate in candidateCaps) {
        val advertisedModes = coordinatedHostDisplayModes.filter { mode ->
            mode.width <= candidate.width && mode.height <= candidate.height
        }
        val supportedRefreshUppers = advertisedModes.mapNotNull { mode ->
            val supportsMinimum = runCatching {
                video.areSizeAndRateSupported(
                    mode.width,
                    mode.height,
                    MINIMUM_REFRESH_MILLIHZ.toDouble() / 1_000.0,
                )
            }.getOrDefault(false)
            if (!supportsMinimum) return@mapNotNull null
            runCatching {
                video.getSupportedFrameRatesFor(mode.width, mode.height).upper
            }.getOrNull()
        }
        if (supportedRefreshUppers.size != advertisedModes.size) continue
        val commonRefreshUpper = minOf(
            display.maximumRefreshHertz,
            supportedRefreshUppers.minOrNull() ?: continue,
        )
        if (commonRefreshUpper * 1_000.0 >= MINIMUM_REFRESH_MILLIHZ.toDouble()) {
            return AdvertisableStandardModes(
                maximumWidth = candidate.width,
                maximumHeight = candidate.height,
                maximumRefreshHertz = commonRefreshUpper,
            )
        }
    }
    return null
}

private fun queryDisplayLimits(context: Context): DisplayLimits {
    val display = context.getSystemService(DisplayManager::class.java)
        ?.getDisplay(Display.DEFAULT_DISPLAY)
    val modes = display?.supportedModes.orEmpty()
    val bestMode = modes.maxByOrNull { it.physicalWidth.toLong() * it.physicalHeight }
    val metrics = context.resources.displayMetrics
    val width = (bestMode?.physicalWidth ?: metrics.widthPixels).coerceAtLeast(1)
    val height = (bestMode?.physicalHeight ?: metrics.heightPixels).coerceAtLeast(1)
    val maximumRefresh = modes.asSequence()
        .map { it.refreshRate.toDouble() }
        .filter { it.isFinite() && it > 0.0 }
        .maxOrNull()
        ?: display?.refreshRate?.toDouble()?.takeIf { it.isFinite() && it > 0.0 }
        ?: throw UnsupportedOperationException("Android display reports no valid refresh rate")
    return DisplayLimits(width, height, maximumRefresh)
}

private fun HardwareAccelerationEvidence.selectionRank(): Int = when (this) {
    HardwareAccelerationEvidence.ReportedHardware -> 0
    HardwareAccelerationEvidence.NotReportedByPlatform -> 1
    HardwareAccelerationEvidence.ReportedSoftware -> 2
}

private data class DisplayLimits(
    val width: Int,
    val height: Int,
    val maximumRefreshHertz: Double,
)

private data class AdvertisableStandardModes(
    val maximumWidth: Int,
    val maximumHeight: Int,
    val maximumRefreshHertz: Double,
)

private data class DecoderProbe(
    val decoderName: String,
    val hardwareAcceleration: HardwareAccelerationEvidence,
    val maximumWidth: Int,
    val maximumHeight: Int,
    val maximumRefreshMillihz: UInt,
    val maximumBitrateKbps: UInt,
    val probeWidth: Int,
    val probeHeight: Int,
)

private const val MINIMUM_REFRESH_MILLIHZ = 30_000u
