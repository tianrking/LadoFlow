package dev.ladoflow.display.input

import dev.ladoflow.display.protocol.MAX_TOUCH_CONTACTS

/** Stable Android pointer-id to LDFL contact-id allocation for one gesture. */
class TouchContactTracker {
    private val contacts = linkedMapOf<Int, Int>()

    val activePlatformIds: List<Int>
        get() = contacts.keys.toList()

    fun begin(platformPointerId: Int): Int? {
        contacts[platformPointerId]?.let { return it }
        val available = (0 until MAX_TOUCH_CONTACTS).firstOrNull { it !in contacts.values }
            ?: return null
        contacts[platformPointerId] = available
        return available
    }

    fun contactId(platformPointerId: Int): Int? = contacts[platformPointerId]

    fun end(platformPointerId: Int): Int? = contacts.remove(platformPointerId)

    fun cancelAll(): List<Pair<Int, Int>> = contacts.entries
        .map { it.key to it.value }
        .also { contacts.clear() }
}
