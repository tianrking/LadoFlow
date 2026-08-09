#pragma once

// Portions of the IddCx lifecycle follow Microsoft's Windows-driver-samples
// IndirectDisplay sample, distributed under the MIT License.

#define NOMINMAX
#include <windows.h>
#include <bugcodes.h>
#include <wudfwdm.h>
#include <wdf.h>
#include <iddcx.h>

#include <avrt.h>
#include <d3d11_2.h>
#include <dxgi1_5.h>
#include <wrl.h>

#include <array>
#include <atomic>
#include <cstdint>
#include <memory>

#include "Trace.h"

namespace Microsoft::WRL::Wrappers
{
    using Thread = HandleT<HandleTraits::HANDLENullTraits>;
}

namespace LadoFlow::Idd
{
    struct DisplayMode
    {
        DWORD Width;
        DWORD Height;
        DWORD RefreshRate;
    };

    class Direct3DDevice final
    {
    public:
        explicit Direct3DDevice(LUID adapterLuid) noexcept;
        HRESULT Initialize() noexcept;

        Microsoft::WRL::ComPtr<ID3D11Device> Device;

    private:
        LUID m_adapterLuid{};
        Microsoft::WRL::ComPtr<IDXGIFactory5> m_factory;
        Microsoft::WRL::ComPtr<IDXGIAdapter1> m_adapter;
        Microsoft::WRL::ComPtr<ID3D11DeviceContext> m_context;
    };

    class SwapChainProcessor final
    {
    public:
        SwapChainProcessor(
            IDDCX_SWAPCHAIN swapChain,
            std::shared_ptr<Direct3DDevice> device,
            HANDLE availableBufferEvent) noexcept;
        ~SwapChainProcessor();

        SwapChainProcessor(const SwapChainProcessor&) = delete;
        SwapChainProcessor& operator=(const SwapChainProcessor&) = delete;

        bool IsRunning() const noexcept;

    private:
        static DWORD CALLBACK ThreadEntry(LPVOID context) noexcept;
        void Run() noexcept;
        void ProcessFrames() noexcept;

        IDDCX_SWAPCHAIN m_swapChain{};
        std::shared_ptr<Direct3DDevice> m_device;
        HANDLE m_availableBufferEvent{};
        Microsoft::WRL::Wrappers::Thread m_thread;
        Microsoft::WRL::Wrappers::Event m_stopEvent;
        std::atomic<std::uint64_t> m_framesProcessed{0};
    };

    class MonitorContext final
    {
    public:
        explicit MonitorContext(IDDCX_MONITOR monitor) noexcept;
        ~MonitorContext();

        NTSTATUS AssignSwapChain(
            IDDCX_SWAPCHAIN swapChain,
            LUID renderAdapter,
            HANDLE availableBufferEvent) noexcept;
        void UnassignSwapChain() noexcept;

    private:
        IDDCX_MONITOR m_monitor{};
        std::unique_ptr<SwapChainProcessor> m_processor;
    };

    class DeviceContext final
    {
    public:
        explicit DeviceContext(WDFDEVICE device) noexcept;
        ~DeviceContext() = default;

        NTSTATUS InitializeAdapter() noexcept;
        NTSTATUS ReportMonitor() noexcept;

    private:
        WDFDEVICE m_device{};
        IDDCX_ADAPTER m_adapter{};
        std::atomic<bool> m_adapterInitializationStarted{false};
    };
}
