/*++

Copyright (c) Microsoft Corporation.
Copyright (c) 2026 LadoFlow contributors.

This file derives its IddCx lifecycle from Microsoft's IndirectDisplay sample,
which is distributed under the MIT License. Product-specific monitor identity,
mode policy, diagnostics, ownership, and error handling are LadoFlow work.

Environment: User mode, UMDF 2.

--*/

#include "Driver.h"
#include "Driver.tmh"

#include <algorithm>
#include <cstring>
#include <new>

using Microsoft::WRL::ComPtr;

namespace
{
    using namespace LadoFlow::Idd;

    constexpr std::array<DisplayMode, 18> kDisplayModes{{
        {1920, 1080, 60},
        {2732, 2048, 60},
        {2560, 1600, 60},
        {2560, 1440, 60},
        {2048, 1536, 60},
        {1920, 1200, 60},
        {1600, 1200, 60},
        {1366,  768, 60},
        {1280,  800, 60},
        {1280,  720, 60},
        {1024,  768, 60},
        {1024,  640, 60},
        { 960,  600, 60},
        { 960,  540, 60},
        { 800,  600, 60},
        { 800,  500, 60},
        { 640,  480, 60},
        { 640,  400, 60},
    }};

    // Stable identity for the one logical LadoFlow panel. It prevents Windows
    // from treating every host restart as a newly attached monitor.
    constexpr GUID kMonitorContainerId{
        0x4a608996, 0x69d2, 0x4bc6, {0x80, 0x9c, 0x91, 0x0b, 0x29, 0x40, 0x32, 0x93}};

    void FillSignalInfo(
        DISPLAYCONFIG_VIDEO_SIGNAL_INFO& signal,
        const DisplayMode& mode,
        bool monitorMode) noexcept
    {
        signal.totalSize.cx = signal.activeSize.cx = mode.Width;
        signal.totalSize.cy = signal.activeSize.cy = mode.Height;
        signal.AdditionalSignalInfo.vSyncFreqDivider = monitorMode ? 0 : 1;
        signal.AdditionalSignalInfo.videoStandard = 255;
        signal.vSyncFreq.Numerator = mode.RefreshRate;
        signal.vSyncFreq.Denominator = 1;
        signal.hSyncFreq.Numerator = mode.RefreshRate * mode.Height;
        signal.hSyncFreq.Denominator = 1;
        signal.scanLineOrdering = DISPLAYCONFIG_SCANLINE_ORDERING_PROGRESSIVE;
        signal.pixelRate = static_cast<UINT64>(mode.RefreshRate) * mode.Width * mode.Height;
    }

    IDDCX_MONITOR_MODE MakeMonitorMode(const DisplayMode& source) noexcept
    {
        IDDCX_MONITOR_MODE mode{};
        mode.Size = sizeof(mode);
        mode.Origin = IDDCX_MONITOR_MODE_ORIGIN_DRIVER;
        FillSignalInfo(mode.MonitorVideoSignalInfo, source, true);
        return mode;
    }

    IDDCX_TARGET_MODE MakeTargetMode(const DisplayMode& source) noexcept
    {
        IDDCX_TARGET_MODE mode{};
        mode.Size = sizeof(mode);
        FillSignalInfo(mode.TargetVideoSignalInfo.targetVideoSignalInfo, source, false);
        return mode;
    }
}

extern "C" DRIVER_INITIALIZE DriverEntry;

EVT_WDF_DRIVER_DEVICE_ADD LadoFlowDeviceAdd;
EVT_WDF_DEVICE_D0_ENTRY LadoFlowDeviceD0Entry;
EVT_IDD_CX_ADAPTER_INIT_FINISHED LadoFlowAdapterInitFinished;
EVT_IDD_CX_ADAPTER_COMMIT_MODES LadoFlowAdapterCommitModes;
EVT_IDD_CX_PARSE_MONITOR_DESCRIPTION LadoFlowParseMonitorDescription;
EVT_IDD_CX_MONITOR_GET_DEFAULT_DESCRIPTION_MODES LadoFlowMonitorGetDefaultModes;
EVT_IDD_CX_MONITOR_QUERY_TARGET_MODES LadoFlowMonitorQueryModes;
EVT_IDD_CX_MONITOR_ASSIGN_SWAPCHAIN LadoFlowMonitorAssignSwapChain;
EVT_IDD_CX_MONITOR_UNASSIGN_SWAPCHAIN LadoFlowMonitorUnassignSwapChain;

struct DeviceContextWrapper
{
    LadoFlow::Idd::DeviceContext* Context{};

    void Cleanup() noexcept
    {
        delete Context;
        Context = nullptr;
    }
};

struct MonitorContextWrapper
{
    LadoFlow::Idd::MonitorContext* Context{};

    void Cleanup() noexcept
    {
        delete Context;
        Context = nullptr;
    }
};

WDF_DECLARE_CONTEXT_TYPE(DeviceContextWrapper);
WDF_DECLARE_CONTEXT_TYPE(MonitorContextWrapper);

extern "C" BOOL WINAPI DllMain(
    _In_ HINSTANCE instance,
    _In_ UINT reason,
    _In_opt_ LPVOID reserved)
{
    UNREFERENCED_PARAMETER(instance);
    UNREFERENCED_PARAMETER(reason);
    UNREFERENCED_PARAMETER(reserved);
    return TRUE;
}

_Use_decl_annotations_
extern "C" NTSTATUS DriverEntry(PDRIVER_OBJECT driverObject, PUNICODE_STRING registryPath)
{
    WDF_DRIVER_CONFIG config;
    WDF_DRIVER_CONFIG_INIT(&config, LadoFlowDeviceAdd);

    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT(&attributes);

    return WdfDriverCreate(driverObject, registryPath, &attributes, &config, WDF_NO_HANDLE);
}

_Use_decl_annotations_
NTSTATUS LadoFlowDeviceAdd(WDFDRIVER driver, PWDFDEVICE_INIT deviceInit)
{
    UNREFERENCED_PARAMETER(driver);

    WDF_PNPPOWER_EVENT_CALLBACKS powerCallbacks;
    WDF_PNPPOWER_EVENT_CALLBACKS_INIT(&powerCallbacks);
    powerCallbacks.EvtDeviceD0Entry = LadoFlowDeviceD0Entry;
    WdfDeviceInitSetPnpPowerEventCallbacks(deviceInit, &powerCallbacks);

    IDD_CX_CLIENT_CONFIG iddConfig;
    IDD_CX_CLIENT_CONFIG_INIT(&iddConfig);
    iddConfig.EvtIddCxAdapterInitFinished = LadoFlowAdapterInitFinished;
    iddConfig.EvtIddCxAdapterCommitModes = LadoFlowAdapterCommitModes;
    iddConfig.EvtIddCxParseMonitorDescription = LadoFlowParseMonitorDescription;
    iddConfig.EvtIddCxMonitorGetDefaultDescriptionModes = LadoFlowMonitorGetDefaultModes;
    iddConfig.EvtIddCxMonitorQueryTargetModes = LadoFlowMonitorQueryModes;
    iddConfig.EvtIddCxMonitorAssignSwapChain = LadoFlowMonitorAssignSwapChain;
    iddConfig.EvtIddCxMonitorUnassignSwapChain = LadoFlowMonitorUnassignSwapChain;

    NTSTATUS status = IddCxDeviceInitConfig(deviceInit, &iddConfig);
    if (!NT_SUCCESS(status))
    {
        return status;
    }

    WDF_OBJECT_ATTRIBUTES attributes;
    WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, DeviceContextWrapper);
    attributes.EvtCleanupCallback = [](WDFOBJECT object)
    {
        if (auto* wrapper = WdfObjectGet_DeviceContextWrapper(object))
        {
            wrapper->Cleanup();
        }
    };

    WDFDEVICE device{};
    status = WdfDeviceCreate(&deviceInit, &attributes, &device);
    if (!NT_SUCCESS(status))
    {
        return status;
    }

    status = IddCxDeviceInitialize(device);
    if (!NT_SUCCESS(status))
    {
        return status;
    }

    auto* wrapper = WdfObjectGet_DeviceContextWrapper(device);
    wrapper->Context = new (std::nothrow) LadoFlow::Idd::DeviceContext(device);
    if (wrapper->Context == nullptr)
    {
        return STATUS_INSUFFICIENT_RESOURCES;
    }

    TraceEvents(TRACE_LEVEL_INFORMATION, LADOFLOW_TRACE_DRIVER, "LadoFlow IDD device added");
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS LadoFlowDeviceD0Entry(WDFDEVICE device, WDF_POWER_DEVICE_STATE previousState)
{
    UNREFERENCED_PARAMETER(previousState);
    auto* wrapper = WdfObjectGet_DeviceContextWrapper(device);
    if (wrapper == nullptr || wrapper->Context == nullptr)
    {
        return STATUS_INVALID_DEVICE_STATE;
    }

    return wrapper->Context->InitializeAdapter();
}

namespace LadoFlow::Idd
{
    Direct3DDevice::Direct3DDevice(LUID adapterLuid) noexcept : m_adapterLuid(adapterLuid)
    {
    }

    HRESULT Direct3DDevice::Initialize() noexcept
    {
        HRESULT result = CreateDXGIFactory2(0, IID_PPV_ARGS(&m_factory));
        if (FAILED(result))
        {
            return result;
        }

        result = m_factory->EnumAdapterByLuid(m_adapterLuid, IID_PPV_ARGS(&m_adapter));
        if (FAILED(result))
        {
            return result;
        }

        return D3D11CreateDevice(
            m_adapter.Get(),
            D3D_DRIVER_TYPE_UNKNOWN,
            nullptr,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            nullptr,
            0,
            D3D11_SDK_VERSION,
            &Device,
            nullptr,
            &m_context);
    }

    SwapChainProcessor::SwapChainProcessor(
        IDDCX_SWAPCHAIN swapChain,
        std::shared_ptr<Direct3DDevice> device,
        HANDLE availableBufferEvent) noexcept
        : m_swapChain(swapChain),
          m_device(std::move(device)),
          m_availableBufferEvent(availableBufferEvent)
    {
        m_stopEvent.Attach(CreateEventW(nullptr, TRUE, FALSE, nullptr));
        if (m_stopEvent.Get() != nullptr)
        {
            m_thread.Attach(CreateThread(nullptr, 0, ThreadEntry, this, 0, nullptr));
        }
    }

    SwapChainProcessor::~SwapChainProcessor()
    {
        if (m_stopEvent.Get() != nullptr)
        {
            SetEvent(m_stopEvent.Get());
        }
        if (m_thread.Get() != nullptr)
        {
            WaitForSingleObject(m_thread.Get(), INFINITE);
        }
    }

    bool SwapChainProcessor::IsRunning() const noexcept
    {
        return m_thread.Get() != nullptr;
    }

    DWORD CALLBACK SwapChainProcessor::ThreadEntry(LPVOID context) noexcept
    {
        static_cast<SwapChainProcessor*>(context)->Run();
        return 0;
    }

    void SwapChainProcessor::Run() noexcept
    {
        DWORD taskIndex{};
        HANDLE mmcssHandle = AvSetMmThreadCharacteristicsW(L"Distribution", &taskIndex);

        ProcessFrames();

        if (m_swapChain != nullptr)
        {
            WdfObjectDelete(reinterpret_cast<WDFOBJECT>(m_swapChain));
            m_swapChain = nullptr;
        }

        if (mmcssHandle != nullptr)
        {
            AvRevertMmThreadCharacteristics(mmcssHandle);
        }
    }

    void SwapChainProcessor::ProcessFrames() noexcept
    {
        ComPtr<IDXGIDevice> dxgiDevice;
        HRESULT result = m_device->Device.As(&dxgiDevice);
        if (FAILED(result))
        {
            TraceEvents(TRACE_LEVEL_ERROR, LADOFLOW_TRACE_FRAME, "IDXGIDevice query failed: 0x%08x", result);
            return;
        }

        IDARG_IN_SWAPCHAINSETDEVICE setDevice{};
        setDevice.pDevice = dxgiDevice.Get();
        result = IddCxSwapChainSetDevice(m_swapChain, &setDevice);
        if (FAILED(result))
        {
            TraceEvents(TRACE_LEVEL_ERROR, LADOFLOW_TRACE_FRAME, "IddCxSwapChainSetDevice failed: 0x%08x", result);
            return;
        }

        const HANDLE waitHandles[]{m_availableBufferEvent, m_stopEvent.Get()};
        for (;;)
        {
            IDARG_OUT_RELEASEANDACQUIREBUFFER buffer{};
            result = IddCxSwapChainReleaseAndAcquireBuffer(m_swapChain, &buffer);

            if (result == E_PENDING)
            {
                const DWORD waitResult = WaitForMultipleObjects(
                    ARRAYSIZE(waitHandles), waitHandles, FALSE, 16);
                if (waitResult == WAIT_OBJECT_0 || waitResult == WAIT_TIMEOUT)
                {
                    continue;
                }
                if (waitResult == WAIT_OBJECT_0 + 1)
                {
                    break;
                }

                TraceEvents(
                    TRACE_LEVEL_ERROR,
                    LADOFLOW_TRACE_FRAME,
                    "frame wait failed: %lu",
                    waitResult);
                break;
            }

            if (FAILED(result))
            {
                TraceEvents(
                    TRACE_LEVEL_WARNING,
                    LADOFLOW_TRACE_FRAME,
                    "swap chain ended: 0x%08x",
                    result);
                break;
            }

            // DWM owns the frame until the next acquire. The desktop host captures
            // this virtual HMONITOR through Windows.Graphics.Capture, so the IDD's
            // latency-critical job is to acknowledge each surface immediately.
            ComPtr<IDXGIResource> acquiredSurface;
            acquiredSurface.Attach(buffer.MetaData.pSurface);
            acquiredSurface.Reset();

            result = IddCxSwapChainFinishedProcessingFrame(m_swapChain);
            if (FAILED(result))
            {
                TraceEvents(
                    TRACE_LEVEL_ERROR,
                    LADOFLOW_TRACE_FRAME,
                    "IddCxSwapChainFinishedProcessingFrame failed: 0x%08x",
                    result);
                break;
            }

            const auto frameCount = m_framesProcessed.fetch_add(1, std::memory_order_relaxed) + 1;
            if ((frameCount % 600) == 0)
            {
                TraceEvents(
                    TRACE_LEVEL_VERBOSE,
                    LADOFLOW_TRACE_FRAME,
                    "processed %llu frames",
                    frameCount);
            }
        }
    }

    MonitorContext::MonitorContext(IDDCX_MONITOR monitor) noexcept : m_monitor(monitor)
    {
    }

    MonitorContext::~MonitorContext()
    {
        UnassignSwapChain();
    }

    NTSTATUS MonitorContext::AssignSwapChain(
        IDDCX_SWAPCHAIN swapChain,
        LUID renderAdapter,
        HANDLE availableBufferEvent) noexcept
    {
        UnassignSwapChain();
        try
        {
            auto device = std::make_shared<Direct3DDevice>(renderAdapter);
            const HRESULT result = device->Initialize();
            if (FAILED(result))
            {
                TraceEvents(TRACE_LEVEL_ERROR, LADOFLOW_TRACE_FRAME, "D3D11 device creation failed: 0x%08x", result);
                WdfObjectDelete(reinterpret_cast<WDFOBJECT>(swapChain));
                return static_cast<NTSTATUS>(result);
            }

            m_processor = std::make_unique<SwapChainProcessor>(
                swapChain, std::move(device), availableBufferEvent);
            if (!m_processor->IsRunning())
            {
                m_processor.reset();
                WdfObjectDelete(reinterpret_cast<WDFOBJECT>(swapChain));
                return STATUS_INSUFFICIENT_RESOURCES;
            }
        }
        catch (const std::bad_alloc&)
        {
            m_processor.reset();
            WdfObjectDelete(reinterpret_cast<WDFOBJECT>(swapChain));
            return STATUS_INSUFFICIENT_RESOURCES;
        }

        return STATUS_SUCCESS;
    }

    void MonitorContext::UnassignSwapChain() noexcept
    {
        m_processor.reset();
    }

    DeviceContext::DeviceContext(WDFDEVICE device) noexcept : m_device(device)
    {
    }

    NTSTATUS DeviceContext::InitializeAdapter() noexcept
    {
        bool expected = false;
        if (!m_adapterInitializationStarted.compare_exchange_strong(
                expected, true, std::memory_order_acq_rel))
        {
            return STATUS_SUCCESS;
        }

        IDDCX_ADAPTER_CAPS caps{};
        caps.Size = sizeof(caps);
        caps.MaxMonitorsSupported = 1;
        caps.EndPointDiagnostics.Size = sizeof(caps.EndPointDiagnostics);
        caps.EndPointDiagnostics.GammaSupport = IDDCX_FEATURE_IMPLEMENTATION_NONE;
        caps.EndPointDiagnostics.TransmissionType = IDDCX_TRANSMISSION_TYPE_WIRED_OTHER;
        caps.EndPointDiagnostics.pEndPointFriendlyName = L"LadoFlow Virtual Display";
        caps.EndPointDiagnostics.pEndPointManufacturerName = L"LadoFlow Project";
        caps.EndPointDiagnostics.pEndPointModelName = L"LadoFlow USB Display";

        IDDCX_ENDPOINT_VERSION version{};
        version.Size = sizeof(version);
        version.MajorVer = 1;
        version.MinorVer = 0;
        caps.EndPointDiagnostics.pFirmwareVersion = &version;
        caps.EndPointDiagnostics.pHardwareVersion = &version;

        WDF_OBJECT_ATTRIBUTES attributes;
        WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, DeviceContextWrapper);

        IDARG_IN_ADAPTER_INIT input{};
        input.WdfDevice = m_device;
        input.pCaps = &caps;
        input.ObjectAttributes = &attributes;

        IDARG_OUT_ADAPTER_INIT output{};
        const NTSTATUS status = IddCxAdapterInitAsync(&input, &output);
        if (!NT_SUCCESS(status))
        {
            m_adapterInitializationStarted.store(false, std::memory_order_release);
            TraceEvents(TRACE_LEVEL_ERROR, LADOFLOW_TRACE_DEVICE, "IddCxAdapterInitAsync failed: 0x%08x", status);
            return status;
        }

        m_adapter = output.AdapterObject;
        auto* wrapper = WdfObjectGet_DeviceContextWrapper(m_adapter);
        wrapper->Context = this;
        return STATUS_SUCCESS;
    }

    NTSTATUS DeviceContext::ReportMonitor() noexcept
    {
        WDF_OBJECT_ATTRIBUTES attributes;
        WDF_OBJECT_ATTRIBUTES_INIT_CONTEXT_TYPE(&attributes, MonitorContextWrapper);
        attributes.EvtCleanupCallback = [](WDFOBJECT object)
        {
            if (auto* wrapper = WdfObjectGet_MonitorContextWrapper(object))
            {
                wrapper->Cleanup();
            }
        };

        IDDCX_MONITOR_INFO info{};
        info.Size = sizeof(info);
        info.MonitorType = DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INDIRECT_WIRED;
        info.ConnectorIndex = 0;
        info.MonitorDescription.Size = sizeof(info.MonitorDescription);
        info.MonitorDescription.Type = IDDCX_MONITOR_DESCRIPTION_TYPE_EDID;
        info.MonitorDescription.DataSize = 0;
        info.MonitorDescription.pData = nullptr;
        info.MonitorContainerId = kMonitorContainerId;

        IDARG_IN_MONITORCREATE input{};
        input.ObjectAttributes = &attributes;
        input.pMonitorInfo = &info;

        IDARG_OUT_MONITORCREATE output{};
        NTSTATUS status = IddCxMonitorCreate(m_adapter, &input, &output);
        if (!NT_SUCCESS(status))
        {
            return status;
        }

        auto* wrapper = WdfObjectGet_MonitorContextWrapper(output.MonitorObject);
        wrapper->Context = new (std::nothrow) MonitorContext(output.MonitorObject);
        if (wrapper->Context == nullptr)
        {
            WdfObjectDelete(reinterpret_cast<WDFOBJECT>(output.MonitorObject));
            return STATUS_INSUFFICIENT_RESOURCES;
        }

        IDARG_OUT_MONITORARRIVAL arrival{};
        status = IddCxMonitorArrival(output.MonitorObject, &arrival);
        TraceEvents(
            NT_SUCCESS(status) ? TRACE_LEVEL_INFORMATION : TRACE_LEVEL_ERROR,
            LADOFLOW_TRACE_DEVICE,
            "monitor arrival: 0x%08x",
            status);
        return status;
    }
}

_Use_decl_annotations_
NTSTATUS LadoFlowAdapterInitFinished(
    IDDCX_ADAPTER adapterObject,
    const IDARG_IN_ADAPTER_INIT_FINISHED* input)
{
    auto* wrapper = WdfObjectGet_DeviceContextWrapper(adapterObject);
    if (!NT_SUCCESS(input->AdapterInitStatus))
    {
        TraceEvents(
            TRACE_LEVEL_ERROR,
            LADOFLOW_TRACE_DEVICE,
            "adapter initialization failed: 0x%08x",
            input->AdapterInitStatus);
        return input->AdapterInitStatus;
    }
    if (wrapper == nullptr || wrapper->Context == nullptr)
    {
        return STATUS_INVALID_DEVICE_STATE;
    }

    return wrapper->Context->ReportMonitor();
}

_Use_decl_annotations_
NTSTATUS LadoFlowAdapterCommitModes(
    IDDCX_ADAPTER adapterObject,
    const IDARG_IN_COMMITMODES* input)
{
    UNREFERENCED_PARAMETER(adapterObject);
    UNREFERENCED_PARAMETER(input);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS LadoFlowParseMonitorDescription(
    const IDARG_IN_PARSEMONITORDESCRIPTION* input,
    IDARG_OUT_PARSEMONITORDESCRIPTION* output)
{
    UNREFERENCED_PARAMETER(input);
    output->MonitorModeBufferOutputCount = 0;
    output->PreferredMonitorModeIdx = 0;
    return STATUS_INVALID_PARAMETER;
}

_Use_decl_annotations_
NTSTATUS LadoFlowMonitorGetDefaultModes(
    IDDCX_MONITOR monitorObject,
    const IDARG_IN_GETDEFAULTDESCRIPTIONMODES* input,
    IDARG_OUT_GETDEFAULTDESCRIPTIONMODES* output)
{
    UNREFERENCED_PARAMETER(monitorObject);

    output->DefaultMonitorModeBufferOutputCount = static_cast<UINT>(kDisplayModes.size());
    output->PreferredMonitorModeIdx = 0;
    if (input->DefaultMonitorModeBufferInputCount == 0)
    {
        return STATUS_SUCCESS;
    }
    if (input->DefaultMonitorModeBufferInputCount < kDisplayModes.size())
    {
        return STATUS_BUFFER_TOO_SMALL;
    }

    std::transform(
        kDisplayModes.begin(),
        kDisplayModes.end(),
        input->pDefaultMonitorModes,
        MakeMonitorMode);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS LadoFlowMonitorQueryModes(
    IDDCX_MONITOR monitorObject,
    const IDARG_IN_QUERYTARGETMODES* input,
    IDARG_OUT_QUERYTARGETMODES* output)
{
    UNREFERENCED_PARAMETER(monitorObject);

    output->TargetModeBufferOutputCount = static_cast<UINT>(kDisplayModes.size());
    if (input->TargetModeBufferInputCount == 0)
    {
        return STATUS_SUCCESS;
    }
    if (input->TargetModeBufferInputCount < kDisplayModes.size())
    {
        return STATUS_BUFFER_TOO_SMALL;
    }

    std::transform(
        kDisplayModes.begin(),
        kDisplayModes.end(),
        input->pTargetModes,
        MakeTargetMode);
    return STATUS_SUCCESS;
}

_Use_decl_annotations_
NTSTATUS LadoFlowMonitorAssignSwapChain(
    IDDCX_MONITOR monitorObject,
    const IDARG_IN_SETSWAPCHAIN* input)
{
    auto* wrapper = WdfObjectGet_MonitorContextWrapper(monitorObject);
    if (wrapper == nullptr || wrapper->Context == nullptr)
    {
        WdfObjectDelete(reinterpret_cast<WDFOBJECT>(input->hSwapChain));
        return STATUS_INVALID_DEVICE_STATE;
    }

    return wrapper->Context->AssignSwapChain(
        input->hSwapChain,
        input->RenderAdapterLuid,
        input->hNextSurfaceAvailable);
}

_Use_decl_annotations_
NTSTATUS LadoFlowMonitorUnassignSwapChain(IDDCX_MONITOR monitorObject)
{
    auto* wrapper = WdfObjectGet_MonitorContextWrapper(monitorObject);
    if (wrapper != nullptr && wrapper->Context != nullptr)
    {
        wrapper->Context->UnassignSwapChain();
    }
    return STATUS_SUCCESS;
}
