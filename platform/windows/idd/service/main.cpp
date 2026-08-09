#include <windows.h>
#include <sddl.h>
#include <swdevice.h>

#include <array>
#include <cstdint>
#include <cwchar>
#include <iostream>

#include "Protocol.h"

namespace
{
    namespace Ipc = LadoFlow::VirtualDisplay::Ipc;

    constexpr DWORD kClientIoTimeoutMs = 5'000;
    constexpr DWORD kDeviceCreateTimeoutMs = 20'000;
    constexpr DWORD kPipeClientAccess = FILE_READ_DATA | FILE_WRITE_DATA |
                                        FILE_READ_EA | FILE_WRITE_EA |
                                        FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES |
                                        READ_CONTROL | SYNCHRONIZE;
    static_assert(kPipeClientAccess == 0x0012019b);
    constexpr wchar_t kPipeSecurityDescriptor[] =
        L"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x12019b;;;IU)";

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

    class PipeSecurity final
    {
    public:
        PipeSecurity() noexcept = default;
        ~PipeSecurity()
        {
            if (m_descriptor != nullptr)
            {
                LocalFree(m_descriptor);
            }
        }

        PipeSecurity(const PipeSecurity&) = delete;
        PipeSecurity& operator=(const PipeSecurity&) = delete;

        bool Initialize() noexcept
        {
            if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    kPipeSecurityDescriptor,
                    SDDL_REVISION_1,
                    &m_descriptor,
                    nullptr))
            {
                return false;
            }
            if (!IsValidSecurityDescriptor(m_descriptor))
            {
                SetLastError(ERROR_INVALID_SECURITY_DESCR);
                return false;
            }

            m_attributes.nLength = sizeof(m_attributes);
            m_attributes.lpSecurityDescriptor = m_descriptor;
            m_attributes.bInheritHandle = FALSE;
            return true;
        }

        SECURITY_ATTRIBUTES* get() noexcept { return &m_attributes; }

    private:
        PSECURITY_DESCRIPTOR m_descriptor{};
        SECURITY_ATTRIBUTES m_attributes{};
    };

    struct DeviceSnapshot
    {
        Ipc::ServiceState State{Ipc::ServiceState::Ready};
        HRESULT LastError{S_OK};
        std::uint64_t Generation{1};
        std::array<wchar_t, Ipc::kDeviceInstanceIdCapacity> DeviceInstanceId{};
    };

    struct DeviceCreationContext
    {
        HANDLE CompletedEvent{};
        HRESULT Result{E_PENDING};
        std::array<wchar_t, Ipc::kDeviceInstanceIdCapacity> DeviceInstanceId{};
    };

    VOID WINAPI DeviceCreationCallback(
        HSWDEVICE device,
        HRESULT createResult,
        PVOID contextValue,
        PCWSTR deviceInstanceId)
    {
        UNREFERENCED_PARAMETER(device);
        auto* context = static_cast<DeviceCreationContext*>(contextValue);
        context->Result = createResult;
        if (SUCCEEDED(createResult) && deviceInstanceId != nullptr)
        {
            wcsncpy_s(
                context->DeviceInstanceId.data(),
                context->DeviceInstanceId.size(),
                deviceInstanceId,
                _TRUNCATE);
        }
        SetEvent(context->CompletedEvent);
    }

    class DeviceOwner final
    {
    public:
        DeviceOwner() noexcept = default;
        ~DeviceOwner() { Shutdown(); }

        DeviceOwner(const DeviceOwner&) = delete;
        DeviceOwner& operator=(const DeviceOwner&) = delete;

        DeviceSnapshot Snapshot() const noexcept { return m_snapshot; }

        HRESULT Enable(HANDLE serviceStopEvent, bool& changed) noexcept
        {
            changed = false;
            if (m_device != nullptr && m_snapshot.State == Ipc::ServiceState::Enabled)
            {
                m_snapshot.LastError = S_OK;
                return S_OK;
            }

            if (m_device != nullptr)
            {
                CloseDevice();
            }

            SetState(Ipc::ServiceState::Enabling, S_OK);
            UniqueHandle completed(CreateEventW(nullptr, TRUE, FALSE, nullptr));
            if (!completed)
            {
                const HRESULT result = HRESULT_FROM_WIN32(GetLastError());
                SetState(Ipc::ServiceState::Failed, result);
                return result;
            }

            constexpr wchar_t hardwareIds[] = L"LadoFlowVirtualDisplay\0";
            SW_DEVICE_CREATE_INFO createInfo{};
            createInfo.cbSize = sizeof(createInfo);
            createInfo.pszzCompatibleIds = hardwareIds;
            createInfo.pszInstanceId = L"LadoFlowVirtualDisplay";
            createInfo.pszzHardwareIds = hardwareIds;
            createInfo.pszDeviceDescription = L"LadoFlow Virtual Display Adapter";
            createInfo.CapabilityFlags = SWDeviceCapabilitiesRemovable |
                                         SWDeviceCapabilitiesSilentInstall |
                                         SWDeviceCapabilitiesDriverRequired;

            DeviceCreationContext context{};
            context.CompletedEvent = completed.get();
            HSWDEVICE candidate{};
            HRESULT result = SwDeviceCreate(
                L"LadoFlowVirtualDisplay",
                L"HTREE\\ROOT\\0",
                &createInfo,
                0,
                nullptr,
                DeviceCreationCallback,
                &context,
                &candidate);
            if (FAILED(result))
            {
                SetState(Ipc::ServiceState::Failed, result);
                return result;
            }

            m_device = candidate;
            const HANDLE waits[]{completed.get(), serviceStopEvent};
            const DWORD waitResult = WaitForMultipleObjects(
                ARRAYSIZE(waits), waits, FALSE, kDeviceCreateTimeoutMs);
            if (waitResult == WAIT_OBJECT_0)
            {
                result = context.Result;
                if (SUCCEEDED(result))
                {
                    m_snapshot.DeviceInstanceId = context.DeviceInstanceId;
                    SetState(Ipc::ServiceState::Enabled, S_OK);
                    changed = true;
                    return S_OK;
                }
            }
            else if (waitResult == WAIT_OBJECT_0 + 1)
            {
                result = HRESULT_FROM_WIN32(ERROR_CANCELLED);
            }
            else if (waitResult == WAIT_TIMEOUT)
            {
                result = HRESULT_FROM_WIN32(ERROR_TIMEOUT);
            }
            else
            {
                result = HRESULT_FROM_WIN32(GetLastError());
            }

            // SwDeviceClose guarantees that the callback cannot run after it
            // returns, so the stack-owned callback context remains safe here.
            CloseDevice();
            SetState(
                waitResult == WAIT_OBJECT_0 + 1
                    ? Ipc::ServiceState::Stopping
                    : Ipc::ServiceState::Failed,
                result);
            return result;
        }

        HRESULT Disable(bool stopping, bool& changed) noexcept
        {
            changed = m_device != nullptr;
            if (m_device != nullptr)
            {
                SetState(Ipc::ServiceState::Disabling, S_OK);
                CloseDevice();
            }
            else
            {
                ClearDeviceIdentity();
            }

            SetState(
                stopping ? Ipc::ServiceState::Stopping : Ipc::ServiceState::Ready,
                S_OK);
            return S_OK;
        }

        void Shutdown() noexcept
        {
            bool changed{};
            Disable(true, changed);
        }

    private:
        void SetState(Ipc::ServiceState state, HRESULT result) noexcept
        {
            m_snapshot.State = state;
            m_snapshot.LastError = result;
            ++m_snapshot.Generation;
        }

        void ClearDeviceIdentity() noexcept
        {
            m_snapshot.DeviceInstanceId.fill(L'\0');
        }

        void CloseDevice() noexcept
        {
            if (m_device != nullptr)
            {
                SwDeviceClose(m_device);
                m_device = nullptr;
            }
            ClearDeviceIdentity();
        }

        HSWDEVICE m_device{};
        DeviceSnapshot m_snapshot{};
    };

    enum class IoResult
    {
        Complete,
        Stopped,
        TimedOut,
        Failed,
    };

    void CancelAndDrain(HANDLE pipe, OVERLAPPED& operation) noexcept
    {
        CancelIoEx(pipe, &operation);
        DWORD ignored{};
        GetOverlappedResult(pipe, &operation, &ignored, TRUE);
    }

    IoResult WaitForIo(
        HANDLE pipe,
        OVERLAPPED& operation,
        HANDLE ioEvent,
        HANDLE stopEvent,
        DWORD timeout,
        DWORD& transferred) noexcept
    {
        const HANDLE waits[]{stopEvent, ioEvent};
        const DWORD waitResult = WaitForMultipleObjects(ARRAYSIZE(waits), waits, FALSE, timeout);
        if (waitResult == WAIT_OBJECT_0 + 1)
        {
            return GetOverlappedResult(pipe, &operation, &transferred, FALSE)
                       ? IoResult::Complete
                       : IoResult::Failed;
        }

        CancelAndDrain(pipe, operation);
        if (waitResult == WAIT_OBJECT_0)
        {
            return IoResult::Stopped;
        }
        if (waitResult == WAIT_TIMEOUT)
        {
            SetLastError(ERROR_TIMEOUT);
            return IoResult::TimedOut;
        }
        return IoResult::Failed;
    }

    IoResult ReadRequest(
        HANDLE pipe,
        HANDLE stopEvent,
        Ipc::Request& request,
        DWORD& bytesRead) noexcept
    {
        UniqueHandle event(CreateEventW(nullptr, TRUE, FALSE, nullptr));
        if (!event)
        {
            return IoResult::Failed;
        }

        OVERLAPPED operation{};
        operation.hEvent = event.get();
        if (ReadFile(pipe, &request, sizeof(request), &bytesRead, &operation))
        {
            return IoResult::Complete;
        }
        if (GetLastError() != ERROR_IO_PENDING)
        {
            return IoResult::Failed;
        }
        return WaitForIo(
            pipe, operation, event.get(), stopEvent, kClientIoTimeoutMs, bytesRead);
    }

    IoResult WriteResponse(
        HANDLE pipe,
        HANDLE stopEvent,
        const Ipc::Response& response) noexcept
    {
        UniqueHandle event(CreateEventW(nullptr, TRUE, FALSE, nullptr));
        if (!event)
        {
            return IoResult::Failed;
        }

        OVERLAPPED operation{};
        operation.hEvent = event.get();
        DWORD bytesWritten{};
        if (WriteFile(pipe, &response, sizeof(response), &bytesWritten, &operation))
        {
            return bytesWritten == sizeof(response) ? IoResult::Complete : IoResult::Failed;
        }
        if (GetLastError() != ERROR_IO_PENDING)
        {
            return IoResult::Failed;
        }
        const IoResult result = WaitForIo(
            pipe, operation, event.get(), stopEvent, kClientIoTimeoutMs, bytesWritten);
        return result == IoResult::Complete && bytesWritten != sizeof(response)
                   ? IoResult::Failed
                   : result;
    }

    Ipc::Response ExecuteRequest(
        const Ipc::Request& request,
        DeviceOwner& device,
        HANDLE stopEvent) noexcept
    {
        HRESULT result = S_OK;
        bool changed{};
        if (!Ipc::ValidateRequest(request))
        {
            result = HRESULT_FROM_WIN32(ERROR_INVALID_DATA);
        }
        else
        {
            switch (static_cast<Ipc::Command>(request.CommandValue))
            {
            case Ipc::Command::Enable:
                result = device.Enable(stopEvent, changed);
                break;
            case Ipc::Command::Disable:
                result = device.Disable(false, changed);
                break;
            case Ipc::Command::Status:
            case Ipc::Command::Ping:
                break;
            default:
                result = E_NOTIMPL;
                break;
            }
        }

        const DeviceSnapshot snapshot = device.Snapshot();
        Ipc::Response response = Ipc::MakeResponse(
            request,
            snapshot.State,
            static_cast<std::int32_t>(result),
            static_cast<std::int32_t>(snapshot.LastError),
            GetCurrentProcessId(),
            snapshot.Generation,
            changed);
        wcsncpy_s(
            response.DeviceInstanceId,
            ARRAYSIZE(response.DeviceInstanceId),
            snapshot.DeviceInstanceId.data(),
            _TRUNCATE);
        return response;
    }

    class PipeServer final
    {
    public:
        PipeServer(DeviceOwner& device, HANDLE stopEvent) noexcept
            : m_device(device), m_stopEvent(stopEvent)
        {
        }

        DWORD Initialize() noexcept
        {
            if (!m_security.Initialize())
            {
                return GetLastError();
            }

            m_pipe.reset(CreateNamedPipeW(
                Ipc::kPipeName,
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                sizeof(Ipc::Response),
                sizeof(Ipc::Request),
                0,
                m_security.get()));
            return m_pipe ? ERROR_SUCCESS : GetLastError();
        }

        DWORD Run() noexcept
        {
            UniqueHandle connectEvent(CreateEventW(nullptr, TRUE, FALSE, nullptr));
            if (!connectEvent)
            {
                return GetLastError();
            }

            while (WaitForSingleObject(m_stopEvent, 0) == WAIT_TIMEOUT)
            {
                ResetEvent(connectEvent.get());
                OVERLAPPED connection{};
                connection.hEvent = connectEvent.get();
                bool connected = ConnectNamedPipe(m_pipe.get(), &connection) != FALSE;
                if (!connected)
                {
                    const DWORD error = GetLastError();
                    if (error == ERROR_PIPE_CONNECTED)
                    {
                        connected = true;
                    }
                    else if (error == ERROR_IO_PENDING)
                    {
                        DWORD ignored{};
                        const IoResult result = WaitForIo(
                            m_pipe.get(),
                            connection,
                            connectEvent.get(),
                            m_stopEvent,
                            INFINITE,
                            ignored);
                        if (result == IoResult::Stopped)
                        {
                            break;
                        }
                        connected = result == IoResult::Complete;
                    }
                    else
                    {
                        return error;
                    }
                }

                if (connected)
                {
                    ProcessClient();
                }
                DisconnectNamedPipe(m_pipe.get());
            }
            return ERROR_SUCCESS;
        }

    private:
        void ProcessClient() noexcept
        {
            Ipc::Request request{};
            DWORD bytesRead{};
            const IoResult readResult = ReadRequest(
                m_pipe.get(), m_stopEvent, request, bytesRead);
            if (readResult != IoResult::Complete || bytesRead != sizeof(request))
            {
                return;
            }

            const Ipc::Response response = ExecuteRequest(request, m_device, m_stopEvent);
            WriteResponse(m_pipe.get(), m_stopEvent, response);
        }

        DeviceOwner& m_device;
        HANDLE m_stopEvent{};
        PipeSecurity m_security;
        UniqueHandle m_pipe;
    };

    SERVICE_STATUS_HANDLE g_serviceStatusHandle{};
    SERVICE_STATUS g_serviceStatus{};
    HANDLE g_serviceStopEvent{};
    DWORD g_checkpoint{1};

    void ReportServiceStatus(DWORD currentState, DWORD exitCode, DWORD waitHint) noexcept
    {
        g_serviceStatus.dwServiceType = SERVICE_WIN32_OWN_PROCESS;
        g_serviceStatus.dwCurrentState = currentState;
        g_serviceStatus.dwWin32ExitCode = exitCode;
        g_serviceStatus.dwServiceSpecificExitCode = 0;
        g_serviceStatus.dwWaitHint = waitHint;
        g_serviceStatus.dwControlsAccepted = currentState == SERVICE_RUNNING
                                                 ? SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
                                                 : 0;
        g_serviceStatus.dwCheckPoint =
            currentState == SERVICE_START_PENDING || currentState == SERVICE_STOP_PENDING
                ? g_checkpoint++
                : 0;
        if (g_serviceStatusHandle != nullptr)
        {
            SetServiceStatus(g_serviceStatusHandle, &g_serviceStatus);
        }
    }

    DWORD WINAPI ServiceControlHandler(
        DWORD control,
        DWORD eventType,
        PVOID eventData,
        PVOID context) noexcept
    {
        UNREFERENCED_PARAMETER(eventType);
        UNREFERENCED_PARAMETER(eventData);
        UNREFERENCED_PARAMETER(context);
        if (control == SERVICE_CONTROL_STOP || control == SERVICE_CONTROL_SHUTDOWN)
        {
            ReportServiceStatus(SERVICE_STOP_PENDING, ERROR_SUCCESS, 10'000);
            if (g_serviceStopEvent != nullptr)
            {
                SetEvent(g_serviceStopEvent);
            }
            return NO_ERROR;
        }
        if (control == SERVICE_CONTROL_INTERROGATE)
        {
            ReportServiceStatus(
                g_serviceStatus.dwCurrentState,
                g_serviceStatus.dwWin32ExitCode,
                g_serviceStatus.dwWaitHint);
            return NO_ERROR;
        }
        return ERROR_CALL_NOT_IMPLEMENTED;
    }

    VOID WINAPI ServiceMain(DWORD argumentCount, LPWSTR* arguments) noexcept
    {
        UNREFERENCED_PARAMETER(argumentCount);
        UNREFERENCED_PARAMETER(arguments);
        g_serviceStatusHandle = RegisterServiceCtrlHandlerExW(
            Ipc::kServiceName, ServiceControlHandler, nullptr);
        if (g_serviceStatusHandle == nullptr)
        {
            return;
        }

        ReportServiceStatus(SERVICE_START_PENDING, ERROR_SUCCESS, 10'000);
        UniqueHandle stopEvent(CreateEventW(nullptr, TRUE, FALSE, nullptr));
        if (!stopEvent)
        {
            ReportServiceStatus(SERVICE_STOPPED, GetLastError(), 0);
            return;
        }
        g_serviceStopEvent = stopEvent.get();

        DeviceOwner device;
        PipeServer server(device, stopEvent.get());
        const DWORD initializationResult = server.Initialize();
        if (initializationResult != ERROR_SUCCESS)
        {
            g_serviceStopEvent = nullptr;
            ReportServiceStatus(SERVICE_STOPPED, initializationResult, 0);
            return;
        }

        ReportServiceStatus(SERVICE_RUNNING, ERROR_SUCCESS, 0);
        const DWORD serverResult = server.Run();
        ReportServiceStatus(SERVICE_STOP_PENDING, ERROR_SUCCESS, 10'000);
        device.Shutdown();
        g_serviceStopEvent = nullptr;
        ReportServiceStatus(SERVICE_STOPPED, serverResult, 0);
    }

    bool RunSelfTest() noexcept
    {
        const Ipc::Request request = Ipc::MakeRequest(Ipc::Command::Enable, 7);
        if (!Ipc::ValidateRequest(request))
        {
            return false;
        }
        Ipc::Request invalidRequest = request;
        invalidRequest.Reserved = 1;
        if (Ipc::ValidateRequest(invalidRequest))
        {
            return false;
        }
        invalidRequest = request;
        ++invalidRequest.Magic;
        if (Ipc::ValidateRequest(invalidRequest))
        {
            return false;
        }
        invalidRequest = request;
        ++invalidRequest.Version;
        if (Ipc::ValidateRequest(invalidRequest))
        {
            return false;
        }
        invalidRequest = request;
        --invalidRequest.Size;
        if (Ipc::ValidateRequest(invalidRequest))
        {
            return false;
        }
        invalidRequest = request;
        invalidRequest.CommandValue = 99;
        if (Ipc::ValidateRequest(invalidRequest))
        {
            return false;
        }
        invalidRequest = request;
        invalidRequest.CorrelationId = 0;
        if (Ipc::ValidateRequest(invalidRequest))
        {
            return false;
        }
        const Ipc::Response response = Ipc::MakeResponse(
            request, Ipc::ServiceState::Ready, S_OK, S_OK, 42, 9, false);
        if (!Ipc::ValidateResponse(request, response))
        {
            return false;
        }

        PipeSecurity security;
        if (!security.Initialize())
        {
            return false;
        }
        std::array<wchar_t, 128> testPipeName{};
        swprintf_s(
            testPipeName.data(),
            testPipeName.size(),
            L"\\\\.\\pipe\\LadoFlow.VirtualDisplay.SelfTest.%lu",
            GetCurrentProcessId());
        UniqueHandle pipe(CreateNamedPipeW(
            testPipeName.data(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            sizeof(Ipc::Response),
            sizeof(Ipc::Request),
            0,
            security.get()));
        if (!pipe)
        {
            return false;
        }
        UniqueHandle client(CreateFileW(
            testPipeName.data(),
            kPipeClientAccess,
            0,
            nullptr,
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            nullptr));
        if (!client)
        {
            return false;
        }
        ULONG serverProcessId{};
        if (!GetNamedPipeServerProcessId(client.get(), &serverProcessId) ||
            serverProcessId != GetCurrentProcessId())
        {
            return false;
        }
        DWORD flags{};
        if (!GetNamedPipeInfo(pipe.get(), &flags, nullptr, nullptr, nullptr))
        {
            return false;
        }
        return (flags & PIPE_REJECT_REMOTE_CLIENTS) != 0 &&
               (flags & PIPE_TYPE_MESSAGE) != 0;
    }
}

int wmain(int argumentCount, wchar_t* arguments[])
{
    if (argumentCount == 2 && wcscmp(arguments[1], L"self-test") == 0)
    {
        const bool passed = RunSelfTest();
        std::wcout << L"{\"ok\":" << (passed ? L"true" : L"false")
                   << L",\"protocolVersion\":" << static_cast<unsigned>(Ipc::kVersion)
                   << L"}\n";
        return passed ? 0 : 10;
    }
    if (argumentCount > 2 ||
        (argumentCount == 2 && wcscmp(arguments[1], L"service") != 0))
    {
        std::wcerr << L"Usage: LadoFlowDisplayService [service|self-test]\n";
        return 2;
    }

    SERVICE_TABLE_ENTRYW table[]{
        {const_cast<LPWSTR>(Ipc::kServiceName), ServiceMain},
        {nullptr, nullptr},
    };
    if (!StartServiceCtrlDispatcherW(table))
    {
        return static_cast<int>(GetLastError());
    }
    return 0;
}
