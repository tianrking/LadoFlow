#pragma once

// Trace GUID: a196c787-8ce0-4852-a7a6-4edf3be6271c
#define WPP_CONTROL_GUIDS                                                      \
    WPP_DEFINE_CONTROL_GUID(                                                   \
        LadoFlowIddTraceGuid, (a196c787,8ce0,4852,a7a6,4edf3be6271c),          \
        WPP_DEFINE_BIT(LADOFLOW_TRACE_ALL)                                     \
        WPP_DEFINE_BIT(LADOFLOW_TRACE_DRIVER)                                  \
        WPP_DEFINE_BIT(LADOFLOW_TRACE_DEVICE)                                  \
        WPP_DEFINE_BIT(LADOFLOW_TRACE_FRAME))

#define WPP_FLAG_LEVEL_LOGGER(flag, level) WPP_LEVEL_LOGGER(flag)
#define WPP_FLAG_LEVEL_ENABLED(flag, level)                                    \
    (WPP_LEVEL_ENABLED(flag) && WPP_CONTROL(WPP_BIT_##flag).Level >= level)

#define WPP_LEVEL_FLAGS_LOGGER(level, flags) WPP_LEVEL_LOGGER(flags)
#define WPP_LEVEL_FLAGS_ENABLED(level, flags)                                  \
    (WPP_LEVEL_ENABLED(flags) && WPP_CONTROL(WPP_BIT_##flags).Level >= level)

// begin_wpp config
// FUNC Trace{FLAG=LADOFLOW_TRACE_ALL}(LEVEL, MSG, ...);
// FUNC TraceEvents(LEVEL, FLAGS, MSG, ...);
// end_wpp

#define MYDRIVER_TRACING_ID L"LadoFlow\\UMDF2.25\\LadoFlowIdd v0.1"
