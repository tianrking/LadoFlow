package dev.ladoflow.display.media

/** A structural error in an H.264 Annex-B access unit. */
class H264AnnexBException(message: String) : IllegalArgumentException(message)

/**
 * Codec-specific data needed to start a fresh H.264 decoder.
 *
 * The byte arrays contain complete NAL units without an Annex-B start code.
 */
class H264ParameterSets(
    sequenceParameterSet: ByteArray,
    pictureParameterSet: ByteArray,
) {
    val sequenceParameterSet: ByteArray = sequenceParameterSet.copyOf()
    val pictureParameterSet: ByteArray = pictureParameterSet.copyOf()

    init {
        require(this.sequenceParameterSet.isNotEmpty()) { "SPS must not be empty" }
        require(this.pictureParameterSet.isNotEmpty()) { "PPS must not be empty" }
        require(this.sequenceParameterSet[0].toInt() and NAL_TYPE_MASK == NAL_TYPE_SPS) {
            "SPS NAL unit has the wrong type"
        }
        require(this.pictureParameterSet[0].toInt() and NAL_TYPE_MASK == NAL_TYPE_PPS) {
            "PPS NAL unit has the wrong type"
        }
    }

    fun sequenceParameterSetCsd(): ByteArray = sequenceParameterSet.withAnnexBStartCode()

    fun pictureParameterSetCsd(): ByteArray = pictureParameterSet.withAnnexBStartCode()

    override fun equals(other: Any?): Boolean =
        other is H264ParameterSets &&
            sequenceParameterSet.contentEquals(other.sequenceParameterSet) &&
            pictureParameterSet.contentEquals(other.pictureParameterSet)

    override fun hashCode(): Int =
        31 * sequenceParameterSet.contentHashCode() + pictureParameterSet.contentHashCode()
}

data class H264AnnexBInspection(
    val nalUnitTypes: List<Int>,
    val sequenceParameterSet: ByteArray?,
    val pictureParameterSet: ByteArray?,
    val containsVcl: Boolean,
    val containsIdr: Boolean,
)

/** Strict, allocation-bounded inspection of one LDFL H.264 access unit. */
object H264AnnexB {
    fun inspect(accessUnit: ByteArray): H264AnnexBInspection {
        if (accessUnit.isEmpty()) {
            throw H264AnnexBException("H.264 access unit is empty")
        }

        val firstStartCode = findStartCode(accessUnit, 0)
            ?: throw H264AnnexBException("H.264 access unit has no Annex-B start code")
        if (firstStartCode.offset != 0) {
            throw H264AnnexBException("H.264 Annex-B access unit has bytes before its first start code")
        }

        val nalTypes = mutableListOf<Int>()
        var sequenceParameterSet: ByteArray? = null
        var pictureParameterSet: ByteArray? = null
        var containsVcl = false
        var containsIdr = false
        var startCode = firstStartCode

        while (true) {
            val nalStart = startCode.offset + startCode.length
            val nextStartCode = findStartCode(accessUnit, nalStart)
            val nalEnd = nextStartCode?.offset ?: accessUnit.size
            if (nalStart >= nalEnd) {
                throw H264AnnexBException("H.264 Annex-B stream contains an empty NAL unit")
            }

            val header = accessUnit[nalStart].toInt() and 0xff
            if (header and FORBIDDEN_ZERO_BIT != 0) {
                throw H264AnnexBException("H.264 NAL unit sets forbidden_zero_bit")
            }
            val nalType = header and NAL_TYPE_MASK
            if (nalType == 0) {
                throw H264AnnexBException("H.264 NAL unit type zero is unspecified")
            }

            nalTypes += nalType
            when (nalType) {
                NAL_TYPE_SPS -> sequenceParameterSet = accessUnit.copyOfRange(nalStart, nalEnd)
                NAL_TYPE_PPS -> pictureParameterSet = accessUnit.copyOfRange(nalStart, nalEnd)
            }
            if (nalType in NAL_TYPE_NON_IDR_SLICE..NAL_TYPE_IDR_SLICE) {
                containsVcl = true
            }
            if (nalType == NAL_TYPE_IDR_SLICE) {
                containsIdr = true
            }

            startCode = nextStartCode ?: break
        }

        return H264AnnexBInspection(
            nalUnitTypes = nalTypes,
            sequenceParameterSet = sequenceParameterSet,
            pictureParameterSet = pictureParameterSet,
            containsVcl = containsVcl,
            containsIdr = containsIdr,
        )
    }

    private fun findStartCode(
        bytes: ByteArray,
        fromIndex: Int,
    ): StartCode? {
        var index = fromIndex.coerceAtLeast(0)
        while (index + 3 <= bytes.size) {
            if (
                index + 4 <= bytes.size &&
                bytes[index] == 0.toByte() &&
                bytes[index + 1] == 0.toByte() &&
                bytes[index + 2] == 0.toByte() &&
                bytes[index + 3] == 1.toByte()
            ) {
                return StartCode(index, 4)
            }
            if (
                bytes[index] == 0.toByte() &&
                bytes[index + 1] == 0.toByte() &&
                bytes[index + 2] == 1.toByte()
            ) {
                return StartCode(index, 3)
            }
            index += 1
        }
        return null
    }

    private data class StartCode(
        val offset: Int,
        val length: Int,
    )
}

private fun ByteArray.withAnnexBStartCode(): ByteArray =
    ANNEX_B_START_CODE + this

private const val FORBIDDEN_ZERO_BIT = 0x80
private const val NAL_TYPE_MASK = 0x1f
private const val NAL_TYPE_NON_IDR_SLICE = 1
private const val NAL_TYPE_IDR_SLICE = 5
private const val NAL_TYPE_SPS = 7
private const val NAL_TYPE_PPS = 8
private val ANNEX_B_START_CODE = byteArrayOf(0, 0, 0, 1)
