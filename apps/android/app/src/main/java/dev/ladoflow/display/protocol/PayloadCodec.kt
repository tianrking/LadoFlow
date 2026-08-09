package dev.ladoflow.display.protocol

object PayloadCodec {
    fun decode(
        messageType: MessageType,
        payload: ByteArray,
    ): LdflPayload = when (messageType) {
        MessageType.Hello -> HelloPayload.decode(payload)
        MessageType.Capabilities -> CapabilitiesPayload.decode(payload)
        MessageType.DisplayConfig -> DisplayConfigPayload.decode(payload)
        MessageType.VideoFrame -> VideoFramePayload.decode(payload)
        MessageType.Input -> InputPayload.decode(payload)
        MessageType.Telemetry -> TelemetryPayload.decode(payload)
        MessageType.Ping -> PingPayload.decode(payload)
        MessageType.Pong -> PongPayload.decode(payload)
        MessageType.Error -> RemoteErrorPayload.decode(payload)
    }

    inline fun <reified T : LdflPayload> decodeAs(frame: LdflFrame): T {
        val expected = when (T::class) {
            HelloPayload::class -> MessageType.Hello
            CapabilitiesPayload::class -> MessageType.Capabilities
            DisplayConfigPayload::class -> MessageType.DisplayConfig
            VideoFramePayload::class -> MessageType.VideoFrame
            InputPayload::class -> MessageType.Input
            TelemetryPayload::class -> MessageType.Telemetry
            PingPayload::class -> MessageType.Ping
            PongPayload::class -> MessageType.Pong
            RemoteErrorPayload::class -> MessageType.Error
            else -> protocolFailure(
                ProtocolViolation.UnexpectedMessageType,
                "Unsupported typed payload ${T::class.simpleName}",
            )
        }
        if (frame.messageType != expected) {
            protocolFailure(
                ProtocolViolation.UnexpectedMessageType,
                "Expected ${expected.name}, received ${frame.messageType.name}",
            )
        }
        return decode(frame.messageType, frame.payload) as T
    }
}
