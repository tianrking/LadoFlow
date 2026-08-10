package dev.ladoflow.display.transport.tether

import java.net.Inet4Address
import java.net.InetAddress
import java.net.NetworkInterface

internal data class UsbTetherInterfaceCandidate(
    val name: String,
    val isUp: Boolean,
    val isLoopback: Boolean,
    val addresses: List<InetAddress>,
)

/**
 * Fail-safe selection: only an IPv4 address on an interface whose kernel name explicitly looks
 * like USB RNDIS/NCM/ECM is eligible. Wi-Fi, cellular, Ethernet, VPN, wildcard, and loopback are
 * intentionally excluded.
 */
internal fun selectUsbTetherAddress(
    candidates: List<UsbTetherInterfaceCandidate>,
): Inet4Address? = candidates
    .asSequence()
    .filter { it.isUp && !it.isLoopback && it.name.hasUsbTetherPrefix() }
    .flatMap { candidate ->
        candidate.addresses.asSequence().map { address -> candidate.name to address }
    }
    .filter { (_, address) ->
        address is Inet4Address &&
            !address.isAnyLocalAddress &&
            !address.isLoopbackAddress &&
            (address.isSiteLocalAddress || address.isLinkLocalAddress)
    }
    .sortedWith(
        compareBy<Pair<String, InetAddress>>(
            { (name, _) ->
                val normalized = name.lowercase()
                USB_TETHER_INTERFACE_PREFIXES.indexOfFirst(normalized::startsWith)
            },
            { (_, address) -> if (address.isSiteLocalAddress) 0 else 1 },
            { (name, _) -> name },
            { (_, address) -> address.hostAddress },
        ),
    )
    .map { (_, address) -> address as Inet4Address }
    .firstOrNull()

internal fun discoverUsbTetherAddress(): Inet4Address? {
    val interfaces = NetworkInterface.getNetworkInterfaces()?.toList().orEmpty()
    return selectUsbTetherAddress(
        interfaces.map { network ->
            UsbTetherInterfaceCandidate(
                name = network.name.orEmpty(),
                isUp = network.isUp,
                isLoopback = network.isLoopback,
                addresses = network.inetAddresses.toList(),
            )
        },
    )
}

private fun String.hasUsbTetherPrefix(): Boolean {
    val normalized = lowercase()
    return USB_TETHER_INTERFACE_PREFIXES.any(normalized::startsWith)
}

private val USB_TETHER_INTERFACE_PREFIXES = listOf("rndis", "ncm", "ecm", "usb")
