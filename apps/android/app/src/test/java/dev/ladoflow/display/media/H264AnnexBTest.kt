package dev.ladoflow.display.media

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class H264AnnexBTest {
    @Test
    fun inspectsMixedStartCodesAndExtractsParameterSets() {
        val accessUnit = byteArrayOf(
            0, 0, 0, 1, 0x67, 0x42, 0x00, 0x1f,
            0, 0, 1, 0x68, 0x11, 0x22,
            0, 0, 0, 1, 0x65, 0x33, 0x44,
        )

        val inspection = H264AnnexB.inspect(accessUnit)

        assertEquals(listOf(7, 8, 5), inspection.nalUnitTypes)
        assertArrayEquals(byteArrayOf(0x67, 0x42, 0x00, 0x1f), inspection.sequenceParameterSet)
        assertArrayEquals(byteArrayOf(0x68, 0x11, 0x22), inspection.pictureParameterSet)
        assertTrue(inspection.containsVcl)
        assertTrue(inspection.containsIdr)
    }

    @Test
    fun recognizesNonIdrVcl() {
        val inspection = H264AnnexB.inspect(byteArrayOf(0, 0, 1, 0x41, 1, 2, 3))

        assertTrue(inspection.containsVcl)
        assertFalse(inspection.containsIdr)
    }

    @Test
    fun parameterSetsProduceMediaCodecCsdWithFourByteStartCodes() {
        val parameterSets = H264ParameterSets(
            sequenceParameterSet = byteArrayOf(0x67, 0x01),
            pictureParameterSet = byteArrayOf(0x68, 0x02),
        )

        assertArrayEquals(
            byteArrayOf(0, 0, 0, 1, 0x67, 0x01),
            parameterSets.sequenceParameterSetCsd(),
        )
        assertArrayEquals(
            byteArrayOf(0, 0, 0, 1, 0x68, 0x02),
            parameterSets.pictureParameterSetCsd(),
        )
    }

    @Test
    fun rejectsNonAnnexBAndMalformedNalUnits() {
        assertThrows(H264AnnexBException::class.java) {
            H264AnnexB.inspect(byteArrayOf(0x65, 1, 2, 3))
        }
        assertThrows(H264AnnexBException::class.java) {
            H264AnnexB.inspect(byteArrayOf(9, 0, 0, 1, 0x65))
        }
        assertThrows(H264AnnexBException::class.java) {
            H264AnnexB.inspect(byteArrayOf(0, 0, 0, 1))
        }
        assertThrows(H264AnnexBException::class.java) {
            H264AnnexB.inspect(byteArrayOf(0, 0, 1, 0xe5.toByte(), 1))
        }
    }
}
