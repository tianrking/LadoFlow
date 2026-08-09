#include <windows.h>

#include <array>
#include <cstdint>
#include <cwchar>
#include <iomanip>
#include <iostream>
#include <sstream>
#include <string>
#include <string_view>
#include <utility>

#include "Protocol.h"

namespace
{
    namespace Ipc = LadoFlow::VirtualDisplay::Ipc;

    constexpr DWORD kServiceStartTimeoutMs = 15'000;
    constexpr DWORD kPipeWaitTimeoutMs = 5'000;
    constexpr DWORD kPipeClientAccess = FILE_READ_DATA | FILE_WRITE_DATA |
                                        FILE_READ_EA | FILE_WRITE_EA |
                                        FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES |
                                        READ_CONTROL | SYNCHRONIZE;
    static_assert(kPipeClientAccess == 0x0012019b);

    class UniqueHandle final
    {
    public:
        UniqueHandle() noexcept = default;
        explicit UniqueHandle(HANDLE value) noexcept : m_value(value) {}
        ~UniqueHandle() { reset(); }

        UniqueHandle(const UniqueHandle&) = delete;
        UniqueHandle& operator=(const UniqueHandle&) = delete;

        UniqueHandle(UniqueHandle&& other) noexcept : m_value(other.release()) {}
        UniqueHandle& operator=(UniqueHandle&& other) noexcept
        {
            if (this != &other)
            {
                reset(other.release());
            }
            return *this;
        }

        HANDLE get() const noexcept { return m_value; }
        explicit operator bool() const noexcept
        {
            return m_value != nullptr && m_value != INVALID_HANDLE_VALUE;
        }
        HANDLE release() noexcept
        {
            HANDLE value = m_value;
            m_value = nullptr;
            return value;
        }
        void reset(HANDLE value = nullptr) noexcept
        {
            if (*this)
            {
                CloseHandle(m_value);
            }
            m_value = value;
        }

    private:
        HANDLE m_value{};
    };

    class UniqueServiceHandle final
    {
    public:
        UniqueServiceHandle() noexcept = default;
        explicit UniqueServiceHandle(SC_HANDLE value) noexcept : m_value(value) {}
        ~UniqueServiceHandle()
        {
            if (m_value != nullptr)
            {
                CloseServiceHandle(m_value);
            }
        }

        UniqueServiceHandle(const UniqueServiceHandle&) = delete;
        UniqueServiceHandle& operator=(const UniqueServiceHandle&) = delete;

        UniqueServiceHandle(UniqueServiceHandle&& other) noexcept
            : m_value(other.release())
        {
        }
        UniqueServiceHandle& operator=(UniqueServiceHandle&& other) noexcept
        {
            if (this != &other)
            {
                reset(other.release());
            }
            return *this;
        }

        SC_HANDLE get() const noexcept { return m_value; }
        explicit operator bool() const noexcept { return m_value != nullptr; }

        SC_HANDLE release() noexcept
        {
            SC_HANDLE value = m_value;
            m_value = nullptr;
            return value;
        }
        void reset(SC_HANDLE value = nullptr) noexcept
        {
            if (m_value != nullptr)
            {
                CloseServiceHandle(m_value);
            }
            m_value = value;
        }

    private:
        SC_HANDLE m_value{};
    };

    struct ServiceSnapshot
    {
        bool Installed{};
        DWORD State{SERVICE_STOPPED};
        DWORD ProcessId{};
        DWORD Win32ExitCode{};
    };

    std::wstring EscapeJson(std::wstring_view value)
    {
        std::wostringstream output;
        for (const wchar_t character : value)
        {
            switch (character)
            {
            case L'\\': output << L"\\\\"; break;
            case L'\"': output << L"\\\""; break;
            case L'\b': output << L"\\b"; break;
            case L'\f': output << L"\\f"; break;
            case L'\n': output << L"\\n"; break;
            case L'\r': output << L"\\r"; break;
            case L'\t': output << L"\\t"; break;
            default:
                if (character < 0x20)
                {
                    output << L"\\u" << std::hex << std::setw(4) << std::setfill(L'0')
                           << static_cast<unsigned>(character) << std::dec;
                }
                else
                {
                    output << character;
                }
            }
        }
        return output.str();
    }

    std::wstring FormatResult(std::int32_t result)
    {
        std::wostringstream output;
        output << L"0x" << std::uppercase << std::hex << std::setw(8)
               << std::setfill(L'0') << static_cast<std::uint32_t>(result);
        return output.str();
    }

    const wchar_t* ScmStateName(DWORD state) noexcept
    {
        switch (state)
        {
        case SERVICE_START_PENDING:
            return L"starting";
        case SERVICE_RUNNING:
            return L"running";
        case SERVICE_STOP_PENDING:
            return L"stopping";
        case SERVICE_PAUSED:
        case SERVICE_PAUSE_PENDING:
            return L"paused";
        case SERVICE_CONTINUE_PENDING:
            return L"continuing";
        default:
            return L"stopped";
        }
    }

    void PrintUnavailable(const ServiceSnapshot& service, HRESULT result, bool changed)
    {
        std::wcout << L"{\"protocolVersion\":" << static_cast<unsigned>(Ipc::kVersion)
                   << L",\"serviceInstalled\":" << (service.Installed ? L"true" : L"false")
                   << L",\"serviceState\":\"" << ScmStateName(service.State)
                   << L"\",\"state\":\"unavailable\",\"pid\":" << service.ProcessId
                   << L",\"requestResult\":\"" << FormatResult(static_cast<std::int32_t>(result))
                   << L"\",\"lastError\":\"" << FormatResult(static_cast<std::int32_t>(result))
                   << L"\",\"deviceInstanceId\":\"\",\"generation\":0,\"changed\":"
                   << (changed ? L"true" : L"false") << L"}\n";
    }

    void PrintResponse(const ServiceSnapshot& service, const Ipc::Response& response)
    {
        const auto state = static_cast<Ipc::ServiceState>(response.StateValue);
        std::wcout << L"{\"protocolVersion\":" << static_cast<unsigned>(response.Version)
                   << L",\"serviceInstalled\":true,\"serviceState\":\""
                   << ScmStateName(service.State)
                   << L"\",\"state\":\"" << Ipc::StateName(state)
                   << L"\",\"pid\":" << response.ServiceProcessId
                   << L",\"requestResult\":\"" << FormatResult(response.RequestResult)
                   << L"\",\"lastError\":\"" << FormatResult(response.LastError)
                   << L"\",\"deviceInstanceId\":\""
                   << EscapeJson(response.DeviceInstanceId)
                   << L"\",\"generation\":" << response.Generation
                   << L",\"changed\":" << (response.Changed != 0 ? L"true" : L"false")
                   << L"}\n";
    }

    bool QueryService(
        DWORD desiredAccess,
        UniqueServiceHandle& manager,
        UniqueServiceHandle& service,
        ServiceSnapshot& snapshot) noexcept
    {
        manager = UniqueServiceHandle(OpenSCManagerW(nullptr, nullptr, SC_MANAGER_CONNECT));
        if (!manager)
        {
            snapshot.Win32ExitCode = GetLastError();
            return false;
        }

        service = UniqueServiceHandle(OpenServiceW(
            manager.get(), Ipc::kServiceName, desiredAccess | SERVICE_QUERY_STATUS));
        if (!service)
        {
            snapshot.Win32ExitCode = GetLastError();
            snapshot.Installed = snapshot.Win32ExitCode != ERROR_SERVICE_DOES_NOT_EXIST;
            return false;
        }
        snapshot.Installed = true;

        SERVICE_STATUS_PROCESS status{};
        DWORD bytesNeeded{};
        if (!QueryServiceStatusEx(
                service.get(),
                SC_STATUS_PROCESS_INFO,
                reinterpret_cast<LPBYTE>(&status),
                sizeof(status),
                &bytesNeeded))
        {
            snapshot.Win32ExitCode = GetLastError();
            return false;
        }
        snapshot.State = status.dwCurrentState;
        snapshot.ProcessId = status.dwProcessId;
        snapshot.Win32ExitCode = status.dwWin32ExitCode;
        return true;
    }

    bool RefreshService(SC_HANDLE service, ServiceSnapshot& snapshot) noexcept
    {
        SERVICE_STATUS_PROCESS status{};
        DWORD bytesNeeded{};
        if (!QueryServiceStatusEx(
                service,
                SC_STATUS_PROCESS_INFO,
                reinterpret_cast<LPBYTE>(&status),
                sizeof(status),
                &bytesNeeded))
        {
            snapshot.Win32ExitCode = GetLastError();
            return false;
        }
        snapshot.State = status.dwCurrentState;
        snapshot.ProcessId = status.dwProcessId;
        snapshot.Win32ExitCode = status.dwWin32ExitCode;
        return true;
    }

    HRESULT EnsureServiceRunning(UniqueServiceHandle& service, ServiceSnapshot& snapshot) noexcept
    {
        const ULONGLONG deadline = GetTickCount64() + kServiceStartTimeoutMs;
        bool startAttempted{};
        do
        {
            if (snapshot.State == SERVICE_RUNNING)
            {
                return S_OK;
            }
            if (snapshot.State == SERVICE_STOPPED)
            {
                if (startAttempted)
                {
                    return HRESULT_FROM_WIN32(
                        snapshot.Win32ExitCode == ERROR_SUCCESS
                            ? ERROR_SERVICE_NOT_ACTIVE
                            : snapshot.Win32ExitCode);
                }
                if (!StartServiceW(service.get(), 0, nullptr))
                {
                    const DWORD error = GetLastError();
                    if (error != ERROR_SERVICE_ALREADY_RUNNING)
                    {
                        return HRESULT_FROM_WIN32(error);
                    }
                }
                startAttempted = true;
            }
            Sleep(100);
            if (!RefreshService(service.get(), snapshot))
            {
                return HRESULT_FROM_WIN32(snapshot.Win32ExitCode);
            }
        } while (GetTickCount64() < deadline);
        return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
    }

    std::uint64_t NextCorrelationId() noexcept
    {
        LARGE_INTEGER counter{};
        QueryPerformanceCounter(&counter);
        const auto value = static_cast<std::uint64_t>(counter.QuadPart) ^
                           (static_cast<std::uint64_t>(GetCurrentProcessId()) << 32) ^
                           GetTickCount64();
        return value == 0 ? 1 : value;
    }

    HRESULT CallService(
        Ipc::Command command,
        const ServiceSnapshot& service,
        Ipc::Response& response) noexcept
    {
        if (service.State != SERVICE_RUNNING || service.ProcessId == 0)
        {
            return HRESULT_FROM_WIN32(ERROR_SERVICE_NOT_ACTIVE);
        }
        if (!WaitNamedPipeW(Ipc::kPipeName, kPipeWaitTimeoutMs))
        {
            return HRESULT_FROM_WIN32(GetLastError());
        }

        UniqueHandle pipe(CreateFileW(
            Ipc::kPipeName,
            kPipeClientAccess,
            0,
            nullptr,
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            nullptr));
        if (!pipe)
        {
            return HRESULT_FROM_WIN32(GetLastError());
        }

        ULONG serverProcessId{};
        if (!GetNamedPipeServerProcessId(pipe.get(), &serverProcessId))
        {
            return HRESULT_FROM_WIN32(GetLastError());
        }
        if (serverProcessId != service.ProcessId)
        {
            return E_ACCESSDENIED;
        }

        DWORD mode = PIPE_READMODE_MESSAGE;
        if (!SetNamedPipeHandleState(pipe.get(), &mode, nullptr, nullptr))
        {
            return HRESULT_FROM_WIN32(GetLastError());
        }

        const Ipc::Request request = Ipc::MakeRequest(command, NextCorrelationId());
        DWORD bytesRead{};
        if (!TransactNamedPipe(
                pipe.get(),
                const_cast<Ipc::Request*>(&request),
                sizeof(request),
                &response,
                sizeof(response),
                &bytesRead,
                nullptr))
        {
            return HRESULT_FROM_WIN32(GetLastError());
        }
        if (bytesRead != sizeof(response) || !Ipc::ValidateResponse(request, response))
        {
            return HRESULT_FROM_WIN32(ERROR_INVALID_DATA);
        }
        return S_OK;
    }

    int Execute(Ipc::Command command, bool startService)
    {
        UniqueServiceHandle manager;
        UniqueServiceHandle service;
        ServiceSnapshot snapshot{};
        if (!QueryService(0, manager, service, snapshot))
        {
            const HRESULT result = HRESULT_FROM_WIN32(snapshot.Win32ExitCode);
            PrintUnavailable(snapshot, result, false);
            if (command == Ipc::Command::Status)
            {
                return 0;
            }
            return snapshot.Win32ExitCode == ERROR_SERVICE_DOES_NOT_EXIST ? 20 : 21;
        }

        if (startService)
        {
            if (snapshot.State == SERVICE_STOPPED || snapshot.State == SERVICE_STOP_PENDING)
            {
                UniqueServiceHandle startable(OpenServiceW(
                    manager.get(),
                    Ipc::kServiceName,
                    SERVICE_QUERY_STATUS | SERVICE_START));
                if (!startable)
                {
                    const HRESULT result = HRESULT_FROM_WIN32(GetLastError());
                    PrintUnavailable(snapshot, result, false);
                    return 22;
                }
                service = std::move(startable);
            }
            const HRESULT startResult = EnsureServiceRunning(service, snapshot);
            if (FAILED(startResult))
            {
                PrintUnavailable(snapshot, startResult, false);
                return 22;
            }
        }
        else if (snapshot.State != SERVICE_RUNNING)
        {
            PrintUnavailable(
                snapshot,
                HRESULT_FROM_WIN32(ERROR_SERVICE_NOT_ACTIVE),
                false);
            return command == Ipc::Command::Status ? 0 : 23;
        }

        Ipc::Response response{};
        const HRESULT callResult = CallService(command, snapshot, response);
        if (FAILED(callResult))
        {
            PrintUnavailable(snapshot, callResult, false);
            return 24;
        }
        PrintResponse(snapshot, response);
        return SUCCEEDED(response.RequestResult) ? 0 : 25;
    }

    bool RunSelfTest()
    {
        const Ipc::Request request = Ipc::MakeRequest(Ipc::Command::Status, 17);
        const Ipc::Response response = Ipc::MakeResponse(
            request, Ipc::ServiceState::Enabled, S_OK, S_OK, 123, 8, true);
        if (!Ipc::ValidateRequest(request) || !Ipc::ValidateResponse(request, response))
        {
            return false;
        }
        Ipc::Response invalidResponse = response;
        ++invalidResponse.CorrelationId;
        if (Ipc::ValidateResponse(request, invalidResponse))
        {
            return false;
        }
        invalidResponse = response;
        ++invalidResponse.Magic;
        if (Ipc::ValidateResponse(request, invalidResponse))
        {
            return false;
        }
        invalidResponse = response;
        ++invalidResponse.Version;
        if (Ipc::ValidateResponse(request, invalidResponse))
        {
            return false;
        }
        invalidResponse = response;
        --invalidResponse.Size;
        if (Ipc::ValidateResponse(request, invalidResponse))
        {
            return false;
        }
        invalidResponse = response;
        invalidResponse.StateValue = 99;
        if (Ipc::ValidateResponse(request, invalidResponse))
        {
            return false;
        }
        invalidResponse = response;
        invalidResponse.Changed = 2;
        if (Ipc::ValidateResponse(request, invalidResponse))
        {
            return false;
        }
        invalidResponse = response;
        invalidResponse.Reserved = 1;
        return !Ipc::ValidateResponse(request, invalidResponse) &&
               EscapeJson(L"a\\b\"c\n") == L"a\\\\b\\\"c\\n" &&
               FormatResult(E_ACCESSDENIED) == L"0x80070005";
    }

    void PrintUsage()
    {
        std::wcerr << L"Usage: LadoFlowVirtualDisplay <start|stop|status|self-test>\n";
    }
}

int wmain(int argumentCount, wchar_t* arguments[])
{
    if (argumentCount != 2)
    {
        PrintUsage();
        return 2;
    }

    const std::wstring_view command(arguments[1]);
    if (command == L"start")
    {
        return Execute(Ipc::Command::Enable, true);
    }
    if (command == L"stop" || command == L"remove")
    {
        return Execute(Ipc::Command::Disable, false);
    }
    if (command == L"status")
    {
        return Execute(Ipc::Command::Status, false);
    }
    if (command == L"self-test")
    {
        const bool passed = RunSelfTest();
        std::wcout << L"{\"ok\":" << (passed ? L"true" : L"false")
                   << L",\"protocolVersion\":" << static_cast<unsigned>(Ipc::kVersion)
                   << L"}\n";
        return passed ? 0 : 10;
    }

    PrintUsage();
    return 2;
}
