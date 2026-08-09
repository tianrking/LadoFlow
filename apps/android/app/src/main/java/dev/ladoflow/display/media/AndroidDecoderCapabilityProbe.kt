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
    val maximumDimension = minOf(
        displayLimits.maximumDimension,
        selected.maximumWidth,
        selected.maximumHeight,
        UShort.MAX_VALUE.toInt(),
    )
    return AndroidDisplayCapabilityEvidence(
        capabilities = CapabilitiesPayload(
            maxWidth = maximumDimension,
            maxHeight = maximumDimension,
            maxRefreshMillihz = selected.maximumRefreshMillihz,
            maxBitrateKbps = selected.maximumBitrateKbps,
            codecs = CodecCapabilities.H264,
            input = InputCapabilities.Pointer or InputCapabilities.Touch,
            features = FeatureFlags.DynamicRotation,
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
    val probeSize = findSupportedDisplaySize(video, display.width, display.height) ?: return null
    val codecRefreshUpper = runCatching {
        video.getSupportedFrameRatesFor(probeSize.first, probeSize.second).upper
    }.getOrNull() ?: return null
    val refreshMillihz = floor(
        minOf(display.maximumRefreshHertz, codecRefreshUpper) * 1_000.0,
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
        maximumWidth = video.supportedWidths.upper,
        maximumHeight = video.supportedHeights.upper,
        maximumRefreshMillihz = refreshMillihz,
        maximumBitrateKbps = maximumBitrateKbps,
        probeWidth = probeSize.first,
        probeHeight = probeSize.second,
    )
}

private fun findSupportedDisplaySize(
    video: MediaCodecInfo.VideoCapabilities,
    requestedWidth: Int,
    requestedHeight: Int,
): Pair<Int, Int>? {
    var width = alignDown(
        minOf(requestedWidth, video.supportedWidths.upper),
        video.widthAlignment,
    )
    var height = alignDown(
        minOf(requestedHeight, video.supportedHeights.upper),
        video.heightAlignment,
    )
    repeat(16) {
        if (width > 0 && height > 0 && runCatching { video.isSizeSupported(width, height) }.getOrDefault(false)) {
            return width to height
        }
        width = alignDown(width * 3 / 4, video.widthAlignment)
        height = alignDown(height * 3 / 4, video.heightAlignment)
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

private fun alignDown(value: Int, alignment: Int): Int = value - value % alignment

private fun HardwareAccelerationEvidence.selectionRank(): Int = when (this) {
    HardwareAccelerationEvidence.ReportedHardware -> 0
    HardwareAccelerationEvidence.NotReportedByPlatform -> 1
    HardwareAccelerationEvidence.ReportedSoftware -> 2
}

private data class DisplayLimits(
    val width: Int,
    val height: Int,
    val maximumRefreshHertz: Double,
) {
    val maximumDimension: Int = maxOf(width, height)
}

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
