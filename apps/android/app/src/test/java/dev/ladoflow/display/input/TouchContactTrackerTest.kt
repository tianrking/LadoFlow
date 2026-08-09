package dev.ladoflow.display.input

import dev.ladoflow.display.protocol.MAX_TOUCH_CONTACTS
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TouchContactTrackerTest {
    @Test
    fun allocatesStableBoundedContactIdsAndReusesReleasedSlot() {
        val tracker = TouchContactTracker()

        repeat(MAX_TOUCH_CONTACTS) { pointerId ->
            assertEquals(pointerId, tracker.begin(100 + pointerId))
            assertEquals(pointerId, tracker.contactId(100 + pointerId))
        }
        assertNull(tracker.begin(999))
        assertEquals(4, tracker.end(104))
        assertEquals(4, tracker.begin(999))
    }

    @Test
    fun duplicateBeginIsStableAndCancelReturnsEveryMapping() {
        val tracker = TouchContactTracker()
        assertEquals(0, tracker.begin(7))
        assertEquals(0, tracker.begin(7))
        assertEquals(1, tracker.begin(9))

        assertEquals(listOf(7 to 0, 9 to 1), tracker.cancelAll())
        assertEquals(emptyList<Int>(), tracker.activePlatformIds)
        assertNull(tracker.contactId(7))
    }
}
