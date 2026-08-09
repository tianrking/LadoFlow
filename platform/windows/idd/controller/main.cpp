#include <windows.h>
#include <swdevice.h>

#include <array>
#include <cstdint>
#include <iomanip>
#include <iostream>
#include <sstream>
#include <string>
#include <string_view>

namespace
{
    constexpr wchar_t kMutexName[] = L"Local\\LadoFlow.VirtualDisplay.Controller";
    constexpr wchar_t kStartMutexName[] = L"Local\\LadoFlow.VirtualDisplay.Start";
    constexpr wchar_t kStopEventName[] = L"Local\\LadoFlow.VirtualDisplay.Stop";
    constexpr wchar_t kReadyEventName[] = L"Local\\LadoFlow.VirtualDisplay.Ready";
    constexpr wchar_t kStateMappingName[] = L"Local\\LadoFlow.VirtualDisplay.State";
    constexpr std::uint32_t kStateMagic = 0x4C464944; // LFID
    constexpr std::uint32_t kStateVersion = 1;

    enum class RuntimeState : LONG
    {
        Starting = 1,
        Running = 2,
        Stopping = 3,
        Failed = 4,
    };

    struct SharedState
    {
        std::uint32_t Magic;
        std::uint32_t Version;
        DWORD ProcessId;
        volatile LONG State;
        HRESULT LastError;
        wchar_t DeviceInstanceId[256];
    };

    class UniqueHandle final
    {
    public:
        UniqueHandle() noexcept = default;
        explicit UniqueHandle(HANDLE value) noexcept : m_value(value) {}
        ~UniqueHandle()
        {
            reset();
        }

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

    class StateView final
    {
    public:
        StateView() noexcept = default;
        ~StateView()
        {
            if (m_state != nullptr)
            {
                UnmapViewOfFile(m_state);
            }
        }

        StateView(const StateView&) = delete;
        StateView& operator=(const StateView&) = delete;

        bool create() noexcept
        {
            m_mapping.reset(CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                nullptr,
                PAGE_READWRITE,
                0,
                sizeof(SharedState),
                kStateMappingName));
            return map(FILE_MAP_ALL_ACCESS);
        }

        bool open() noexcept
        {
            m_mapping.reset(OpenFileMappingW(FILE_MAP_READ, FALSE, kStateMappingName));
            return map(FILE_MAP_READ);
        }

        SharedState* get() const noexcept { return m_state; }

    private:
        bool map(DWORD access) noexcept
        {
            if (!m_mapping)
            {
                return false;
            }
            m_state = static_cast<SharedState*>(
                MapViewOfFile(m_mapping.get(), access, 0, 0, sizeof(SharedState)));
            return m_state != nullptr;
        }

        UniqueHandle m_mapping;
        SharedState* m_state{};
    };

    class MutexGuard final
    {
    public:
        bool acquire(PCWSTR name, DWORD timeout) noexcept
        {
            m_mutex.reset(CreateMutexW(nullptr, FALSE, name));
            if (!m_mutex)
            {
                return false;
            }
            const DWORD waitResult = WaitForSingleObject(m_mutex.get(), timeout);
            m_locked = waitResult == WAIT_OBJECT_0 || waitResult == WAIT_ABANDONED;
            return m_locked;
        }

        ~MutexGuard()
        {
            if (m_locked)
            {
                ReleaseMutex(m_mutex.get());
            }
        }

        MutexGuard(const MutexGuard&) = delete;
        MutexGuard& operator=(const MutexGuard&) = delete;
        MutexGuard() noexcept = default;

    private:
        UniqueHandle m_mutex;
        bool m_locked{};
    };

    struct CreationContext
    {
        HANDLE CompletedEvent;
        SharedState* State;
    };

    HANDLE g_stopEvent{};

    const wchar_t* StateName(LONG state) noexcept
    {
        switch (static_cast<RuntimeState>(state))
        {
        case RuntimeState::Starting:
            return L"starting";
        case RuntimeState::Running:
            return L"running";
        case RuntimeState::Stopping:
            return L"stopping";
        case RuntimeState::Failed:
            return L"failed";
        default:
            return L"stopped";
        }
    }

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

    void PrintState(const SharedState* state, bool changed)
    {
        if (state == nullptr || state->Magic != kStateMagic || state->Version != kStateVersion)
        {
            std::wcout << L"{\"state\":\"stopped\",\"pid\":0,\"lastError\":\"0x00000000\","
                          L"\"deviceInstanceId\":\"\",\"changed\":"
                       << (changed ? L"true" : L"false") << L"}\n";
            return;
        }

        std::wostringstream error;
        error << L"0x" << std::uppercase << std::hex << std::setw(8) << std::setfill(L'0')
              << static_cast<unsigned long>(state->LastError);
        std::wcout << L"{\"state\":\"" << StateName(state->State)
                   << L"\",\"pid\":" << state->ProcessId
                   << L",\"lastError\":\"" << error.str()
                   << L"\",\"deviceInstanceId\":\"" << EscapeJson(state->DeviceInstanceId)
                   << L"\",\"changed\":" << (changed ? L"true" : L"false") << L"}\n";
    }

    BOOL WINAPI ConsoleControlHandler(DWORD controlType)
    {
        switch (controlType)
        {
        case CTRL_C_EVENT:
        case CTRL_BREAK_EVENT:
        case CTRL_CLOSE_EVENT:
        case CTRL_LOGOFF_EVENT:
        case CTRL_SHUTDOWN_EVENT:
            if (g_stopEvent != nullptr)
            {
                SetEvent(g_stopEvent);
                return TRUE;
            }
            break;
        default:
            break;
        }
        return FALSE;
    }

    VOID WINAPI CreationCallback(
        HSWDEVICE device,
        HRESULT createResult,
        PVOID contextValue,
        PCWSTR deviceInstanceId)
    {
        UNREFERENCED_PARAMETER(device);
        auto* context = static_cast<CreationContext*>(contextValue);
        context->State->LastError = createResult;
        if (SUCCEEDED(createResult))
        {
            if (deviceInstanceId != nullptr)
            {
                wcsncpy_s(
                    context->State->DeviceInstanceId,
                    deviceInstanceId,
                    _TRUNCATE);
            }
            InterlockedExchange(&context->State->State, static_cast<LONG>(RuntimeState::Running));
        }
        else
        {
            InterlockedExchange(&context->State->State, static_cast<LONG>(RuntimeState::Failed));
        }
        SetEvent(context->CompletedEvent);
    }

    int RunDaemon()
    {
        UniqueHandle ownership(CreateMutexW(nullptr, TRUE, kMutexName));
        if (!ownership)
        {
            return 20;
        }
        if (GetLastError() == ERROR_ALREADY_EXISTS)
        {
            UniqueHandle readyEvent(OpenEventW(EVENT_MODIFY_STATE, FALSE, kReadyEventName));
            if (readyEvent)
            {
                SetEvent(readyEvent.get());
            }
            return 0;
        }

        StateView stateView;
        if (!stateView.create())
        {
            return 21;
        }
        auto* state = stateView.get();
        ZeroMemory(state, sizeof(*state));
        state->Magic = kStateMagic;
        state->Version = kStateVersion;
        state->ProcessId = GetCurrentProcessId();
        state->State = static_cast<LONG>(RuntimeState::Starting);

        UniqueHandle stopEvent(CreateEventW(nullptr, TRUE, FALSE, kStopEventName));
        UniqueHandle readyEvent(CreateEventW(nullptr, TRUE, FALSE, kReadyEventName));
        if (!stopEvent || !readyEvent)
        {
            state->LastError = HRESULT_FROM_WIN32(GetLastError());
            state->State = static_cast<LONG>(RuntimeState::Failed);
            if (readyEvent)
            {
                SetEvent(readyEvent.get());
            }
            return 22;
        }
        ResetEvent(stopEvent.get());
        ResetEvent(readyEvent.get());

        g_stopEvent = stopEvent.get();
        SetConsoleCtrlHandler(ConsoleControlHandler, TRUE);

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

        CreationContext context{readyEvent.get(), state};
        HSWDEVICE softwareDevice{};
        HRESULT result = SwDeviceCreate(
            L"LadoFlowVirtualDisplay",
            L"HTREE\\ROOT\\0",
            &createInfo,
            0,
            nullptr,
            CreationCallback,
            &context,
            &softwareDevice);
        if (FAILED(result))
        {
            state->LastError = result;
            state->State = static_cast<LONG>(RuntimeState::Failed);
            SetEvent(readyEvent.get());
            return 23;
        }

        const DWORD creationWait = WaitForSingleObject(readyEvent.get(), 20'000);
        if (creationWait != WAIT_OBJECT_0 || state->State != static_cast<LONG>(RuntimeState::Running))
        {
            if (creationWait != WAIT_OBJECT_0)
            {
                state->LastError = HRESULT_FROM_WIN32(
                    creationWait == WAIT_TIMEOUT ? ERROR_TIMEOUT : GetLastError());
                state->State = static_cast<LONG>(RuntimeState::Failed);
                SetEvent(readyEvent.get());
            }
            SwDeviceClose(softwareDevice);
            return 24;
        }

        WaitForSingleObject(stopEvent.get(), INFINITE);
        InterlockedExchange(&state->State, static_cast<LONG>(RuntimeState::Stopping));
        SwDeviceClose(softwareDevice);
        SetConsoleCtrlHandler(ConsoleControlHandler, FALSE);
        g_stopEvent = nullptr;
        return 0;
    }

    std::wstring ExecutablePath()
    {
        std::array<wchar_t, 32'768> path{};
        const DWORD length = GetModuleFileNameW(nullptr, path.data(), static_cast<DWORD>(path.size()));
        if (length == 0 || length >= path.size())
        {
            return {};
        }
        return std::wstring(path.data(), length);
    }

    bool ReadCurrentState(StateView& view)
    {
        return view.open() && view.get()->Magic == kStateMagic && view.get()->Version == kStateVersion;
    }

    int Start()
    {
        MutexGuard startGuard;
        if (!startGuard.acquire(kStartMutexName, 30'000))
        {
            return 29;
        }

        StateView existing;
        if (ReadCurrentState(existing) &&
            existing.get()->State != static_cast<LONG>(RuntimeState::Failed))
        {
            PrintState(existing.get(), false);
            return 0;
        }

        UniqueHandle readyEvent(CreateEventW(nullptr, TRUE, FALSE, kReadyEventName));
        if (!readyEvent)
        {
            return 30;
        }
        ResetEvent(readyEvent.get());

        StateView state;
        if (!state.create())
        {
            return 31;
        }
        ZeroMemory(state.get(), sizeof(*state.get()));
        state.get()->Magic = kStateMagic;
        state.get()->Version = kStateVersion;
        state.get()->State = static_cast<LONG>(RuntimeState::Starting);

        const std::wstring executable = ExecutablePath();
        if (executable.empty())
        {
            return 32;
        }
        std::wstring commandLine = L"\"" + executable + L"\" run";
        STARTUPINFOW startup{};
        startup.cb = sizeof(startup);
        PROCESS_INFORMATION process{};
        if (!CreateProcessW(
                executable.c_str(),
                commandLine.data(),
                nullptr,
                nullptr,
                FALSE,
                CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP,
                nullptr,
                nullptr,
                &startup,
                &process))
        {
            return 33;
        }
        UniqueHandle processHandle(process.hProcess);
        UniqueHandle threadHandle(process.hThread);

        const DWORD waitResult = WaitForSingleObject(readyEvent.get(), 25'000);
        if (waitResult != WAIT_OBJECT_0)
        {
            UniqueHandle stopEvent(OpenEventW(EVENT_MODIFY_STATE, FALSE, kStopEventName));
            if (stopEvent)
            {
                SetEvent(stopEvent.get());
            }
            PrintState(state.get(), false);
            return 34;
        }

        PrintState(state.get(), true);
        return state.get()->State == static_cast<LONG>(RuntimeState::Running) ? 0 : 35;
    }

    int Stop()
    {
        UniqueHandle stopEvent(OpenEventW(EVENT_MODIFY_STATE, FALSE, kStopEventName));
        if (!stopEvent)
        {
            PrintState(nullptr, false);
            return 0;
        }
        if (!SetEvent(stopEvent.get()))
        {
            return 40;
        }

        constexpr DWORD timeoutMs = 10'000;
        constexpr DWORD pollMs = 100;
        for (DWORD elapsed = 0; elapsed < timeoutMs; elapsed += pollMs)
        {
            StateView state;
            if (!ReadCurrentState(state))
            {
                PrintState(nullptr, true);
                return 0;
            }
            Sleep(pollMs);
        }

        StateView state;
        if (ReadCurrentState(state))
        {
            PrintState(state.get(), false);
        }
        return 41;
    }

    int Status()
    {
        StateView state;
        if (ReadCurrentState(state))
        {
            PrintState(state.get(), false);
        }
        else
        {
            PrintState(nullptr, false);
        }
        return 0;
    }

    void PrintUsage()
    {
        std::wcerr << L"Usage: LadoFlowVirtualDisplay <start|stop|status|run>\n";
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
        return Start();
    }
    if (command == L"stop" || command == L"remove")
    {
        return Stop();
    }
    if (command == L"status")
    {
        return Status();
    }
    if (command == L"run")
    {
        return RunDaemon();
    }

    PrintUsage();
    return 2;
}
