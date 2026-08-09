package dev.ladoflow.display.input

import android.view.KeyEvent
import dev.ladoflow.display.protocol.KeyModifiers
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AndroidHidUsageMapperTest {
    @Test
    fun mapsLettersNumbersNavigationAndModifierKeys() {
        assertEquals(0x04, AndroidHidUsageMapper.usageForKeyCode(KeyEvent.KEYCODE_A))
        assertEquals(0x1d, AndroidHidUsageMapper.usageForKeyCode(KeyEvent.KEYCODE_Z))
        assertEquals(0x1e, AndroidHidUsageMapper.usageForKeyCode(KeyEvent.KEYCODE_1))
        assertEquals(0x27, AndroidHidUsageMapper.usageForKeyCode(KeyEvent.KEYCODE_0))
        assertEquals(0x28, AndroidHidUsageMapper.usageForKeyCode(KeyEvent.KEYCODE_ENTER))
        assertEquals(0x52, AndroidHidUsageMapper.usageForKeyCode(KeyEvent.KEYCODE_DPAD_UP))
        assertEquals(0xe4, AndroidHidUsageMapper.usageForKeyCode(KeyEvent.KEYCODE_CTRL_RIGHT))
        assertNull(AndroidHidUsageMapper.usageForKeyCode(KeyEvent.KEYCODE_VOLUME_UP))
    }

    @Test
    fun mapsOnlyLdflModifierBits() {
        val modifiers = AndroidHidUsageMapper.modifiers(
            KeyEvent.META_SHIFT_ON or
                KeyEvent.META_CTRL_ON or
                KeyEvent.META_CAPS_LOCK_ON,
        )

        assertTrue(modifiers.contains(KeyModifiers.Shift))
        assertTrue(modifiers.contains(KeyModifiers.Control))
        assertTrue(modifiers.contains(KeyModifiers.CapsLock))
        assertEquals(false, modifiers.contains(KeyModifiers.Alt))
    }
}
