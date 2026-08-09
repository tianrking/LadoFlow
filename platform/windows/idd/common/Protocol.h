#pragma once

#include <cstdint>
#include <type_traits>

namespace LadoFlow::VirtualDisplay::Ipc
{
    inline constexpr wchar_t kServiceName[] = L"LadoFlowVirtualDisplayService";
    inline constexpr wchar_t kServiceDisplayName[] = L"LadoFlow Virtual Display Service";
    inline constexpr wchar_t kPipeName[] = L"\\\\.\\pipe\\LadoFlow.VirtualDisplay.v1";

    inline constexpr std::uint32_t kMagic = 0x4456464c; // LFVD on little-endian Windows.
    inline constexpr std::uint16_t kVersion = 1;
    inline constexpr std::size_t kDeviceInstanceIdCapacity = 260;

    enum class Command : std::uint32_t
    {
        Status = 1,
        Enable = 2,
        Disable = 3,
        Ping = 4,
    };

    enum class ServiceState : std::uint32_t
    {
        Unavailable = 0,
        Starting = 1,
        Ready = 2,
        Enabling = 3,
        Enabled = 4,
        Disabling = 5,
        Failed = 6,
        Stopping = 7,
    };

#pragma pack(push, 1)
    struct Request
    {
        std::uint32_t Magic;
        std::uint16_t Version;
        std::uint16_t Size;
        std::uint32_t CommandValue;
        std::uint32_t Reserved;
        std::uint64_t CorrelationId;
    };

    struct Response
    {
        std::uint32_t Magic;
        std::uint16_t Version;
        std::uint16_t Size;
        std::uint32_t CommandValue;
        std::uint32_t StateValue;
        std::int32_t RequestResult;
        std::int32_t LastError;
        std::uint32_t ServiceProcessId;
        std::uint64_t Generation;
        std::uint64_t CorrelationId;
        std::uint32_t Changed;
        std::uint32_t Reserved;
        wchar_t DeviceInstanceId[kDeviceInstanceIdCapacity];
    };
#pragma pack(pop)

    static_assert(sizeof(wchar_t) == 2, "The IPC protocol is Windows UTF-16 only.");
    static_assert(sizeof(Request) == 24, "Unexpected virtual-display request layout.");
    static_assert(sizeof(Response) == 572, "Unexpected virtual-display response layout.");
    static_assert(std::is_trivially_copyable_v<Request>);
    static_assert(std::is_trivially_copyable_v<Response>);

    constexpr bool IsKnownCommand(std::uint32_t value) noexcept
    {
        return value >= static_cast<std::uint32_t>(Command::Status) &&
               value <= static_cast<std::uint32_t>(Command::Ping);
    }

    constexpr bool IsKnownState(std::uint32_t value) noexcept
    {
        return value <= static_cast<std::uint32_t>(ServiceState::Stopping);
    }

    constexpr Request MakeRequest(Command command, std::uint64_t correlationId) noexcept
    {
        return Request{
            kMagic,
            kVersion,
            static_cast<std::uint16_t>(sizeof(Request)),
            static_cast<std::uint32_t>(command),
            0,
            correlationId,
        };
    }

    constexpr bool ValidateRequest(const Request& request) noexcept
    {
        return request.Magic == kMagic &&
               request.Version == kVersion &&
               request.Size == sizeof(Request) &&
               request.Reserved == 0 &&
               request.CorrelationId != 0 &&
               IsKnownCommand(request.CommandValue);
    }

    constexpr Response MakeResponse(
        const Request& request,
        ServiceState state,
        std::int32_t requestResult,
        std::int32_t lastError,
        std::uint32_t processId,
        std::uint64_t generation,
        bool changed) noexcept
    {
        Response response{};
        response.Magic = kMagic;
        response.Version = kVersion;
        response.Size = static_cast<std::uint16_t>(sizeof(Response));
        response.CommandValue = request.CommandValue;
        response.StateValue = static_cast<std::uint32_t>(state);
        response.RequestResult = requestResult;
        response.LastError = lastError;
        response.ServiceProcessId = processId;
        response.Generation = generation;
        response.CorrelationId = request.CorrelationId;
        response.Changed = changed ? 1u : 0u;
        return response;
    }

    constexpr bool ValidateResponse(const Request& request, const Response& response) noexcept
    {
        return response.Magic == kMagic &&
               response.Version == kVersion &&
               response.Size == sizeof(Response) &&
               response.CommandValue == request.CommandValue &&
               response.CorrelationId == request.CorrelationId &&
               response.Reserved == 0 &&
               response.Changed <= 1 &&
               IsKnownState(response.StateValue);
    }

    constexpr const wchar_t* StateName(ServiceState state) noexcept
    {
        switch (state)
        {
        case ServiceState::Starting:
            return L"starting";
        case ServiceState::Ready:
            return L"ready";
        case ServiceState::Enabling:
            return L"enabling";
        case ServiceState::Enabled:
            return L"enabled";
        case ServiceState::Disabling:
            return L"disabling";
        case ServiceState::Failed:
            return L"failed";
        case ServiceState::Stopping:
            return L"stopping";
        default:
            return L"unavailable";
        }
    }
}
