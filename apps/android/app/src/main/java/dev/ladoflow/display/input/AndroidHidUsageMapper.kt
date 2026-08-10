package dev.ladoflow.display.input

import android.view.KeyEvent
import dev.ladoflow.display.protocol.KeyModifiers

/** Android key codes mapped to USB HID keyboard-page usages used by LDFL v1. */
object AndroidHidUsageMapper {
    fun usageForKeyCode(keyCode: Int): Int? = when (keyCode) {
        in KeyEvent.KEYCODE_A..KeyEvent.KEYCODE_Z -> HID_A + keyCode - KeyEvent.KEYCODE_A
        in KeyEvent.KEYCODE_1..KeyEvent.KEYCODE_9 -> HID_1 + keyCode - KeyEvent.KEYCODE_1
        KeyEvent.KEYCODE_0 -> 0x27
        KeyEvent.KEYCODE_ENTER -> 0x28
        KeyEvent.KEYCODE_ESCAPE -> 0x29
        KeyEvent.KEYCODE_DEL -> 0x2a
        KeyEvent.KEYCODE_TAB -> 0x2b
        KeyEvent.KEYCODE_SPACE -> 0x2c
        KeyEvent.KEYCODE_MINUS -> 0x2d
        KeyEvent.KEYCODE_EQUALS -> 0x2e
        KeyEvent.KEYCODE_LEFT_BRACKET -> 0x2f
        KeyEvent.KEYCODE_RIGHT_BRACKET -> 0x30
        KeyEvent.KEYCODE_BACKSLASH -> 0x31
        KeyEvent.KEYCODE_SEMICOLON -> 0x33
        KeyEvent.KEYCODE_APOSTROPHE -> 0x34
        KeyEvent.KEYCODE_GRAVE -> 0x35
        KeyEvent.KEYCODE_COMMA -> 0x36
        KeyEvent.KEYCODE_PERIOD -> 0x37
        KeyEvent.KEYCODE_SLASH -> 0x38
        KeyEvent.KEYCODE_CAPS_LOCK -> 0x39
        in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12 -> 0x3a + keyCode - KeyEvent.KEYCODE_F1
        KeyEvent.KEYCODE_SYSRQ -> 0x46
        KeyEvent.KEYCODE_SCROLL_LOCK -> 0x47
        KeyEvent.KEYCODE_BREAK -> 0x48
        KeyEvent.KEYCODE_INSERT -> 0x49
        KeyEvent.KEYCODE_MOVE_HOME -> 0x4a
        KeyEvent.KEYCODE_PAGE_UP -> 0x4b
        KeyEvent.KEYCODE_FORWARD_DEL -> 0x4c
        KeyEvent.KEYCODE_MOVE_END -> 0x4d
        KeyEvent.KEYCODE_PAGE_DOWN -> 0x4e
        KeyEvent.KEYCODE_DPAD_RIGHT -> 0x4f
        KeyEvent.KEYCODE_DPAD_LEFT -> 0x50
        KeyEvent.KEYCODE_DPAD_DOWN -> 0x51
        KeyEvent.KEYCODE_DPAD_UP -> 0x52
        KeyEvent.KEYCODE_NUM_LOCK -> 0x53
        KeyEvent.KEYCODE_NUMPAD_DIVIDE -> 0x54
        KeyEvent.KEYCODE_NUMPAD_MULTIPLY -> 0x55
        KeyEvent.KEYCODE_NUMPAD_SUBTRACT -> 0x56
        KeyEvent.KEYCODE_NUMPAD_ADD -> 0x57
        KeyEvent.KEYCODE_NUMPAD_ENTER -> 0x58
        in KeyEvent.KEYCODE_NUMPAD_1..KeyEvent.KEYCODE_NUMPAD_9 ->
            0x59 + keyCode - KeyEvent.KEYCODE_NUMPAD_1

        KeyEvent.KEYCODE_NUMPAD_0 -> 0x62
        KeyEvent.KEYCODE_NUMPAD_DOT -> 0x63
        KeyEvent.KEYCODE_MENU -> 0x65
        KeyEvent.KEYCODE_CTRL_LEFT -> 0xe0
        KeyEvent.KEYCODE_SHIFT_LEFT -> 0xe1
        KeyEvent.KEYCODE_ALT_LEFT -> 0xe2
        KeyEvent.KEYCODE_META_LEFT -> 0xe3
        KeyEvent.KEYCODE_CTRL_RIGHT -> 0xe4
        KeyEvent.KEYCODE_SHIFT_RIGHT -> 0xe5
        KeyEvent.KEYCODE_ALT_RIGHT -> 0xe6
        KeyEvent.KEYCODE_META_RIGHT -> 0xe7
        else -> null
    }

    fun modifiers(metaState: Int): KeyModifiers {
        var bits = 0
        if (metaState and KeyEvent.META_SHIFT_ON != 0) bits = bits or (1 shl 0)
        if (metaState and KeyEvent.META_CTRL_ON != 0) bits = bits or (1 shl 1)
        if (metaState and KeyEvent.META_ALT_ON != 0) bits = bits or (1 shl 2)
        if (metaState and KeyEvent.META_META_ON != 0) bits = bits or (1 shl 3)
        if (metaState and KeyEvent.META_CAPS_LOCK_ON != 0) bits = bits or (1 shl 4)
        if (metaState and KeyEvent.META_NUM_LOCK_ON != 0) bits = bits or (1 shl 5)
        return KeyModifiers.fromBits(bits)
    }

    private const val HID_A = 0x04
    private const val HID_1 = 0x1e
}
