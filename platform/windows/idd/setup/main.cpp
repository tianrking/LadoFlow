#include <windows.h>

#include <bcrypt.h>
#include <newdev.h>
#include <setupapi.h>

#include <algorithm>
#include <array>
#include <cctype>
#include <cstdint>
#include <iostream>
#include <memory>
#include <string>
#include <vector>

#include "../common/Protocol.h"

namespace
{
    namespace Ipc = LadoFlow::VirtualDisplay::Ipc;

    constexpr wchar_t kRegistryPath[] = L"SOFTWARE\\LadoFlow\\WindowsHost";
    constexpr wchar_t kDriversRegistryPath[] = L"SOFTWARE\\LadoFlow\\WindowsHost\\Drivers";
    constexpr wchar_t kServiceBinaryName[] = L"LadoFlowDisplayService.exe";
    constexpr wchar_t kControllerBinaryName[] = L"LadoFlowVirtualDisplay.exe";
    constexpr wchar_t kDriverInfRelativePath[] = L"driver\\LadoFlowIdd.inf";
    constexpr wchar_t kDriverDllRelativePath[] = L"driver\\LadoFlowIdd.dll";
    constexpr wchar_t kDriverCatalogRelativePath[] = L"driver\\ladoflowidd.cat";
    constexpr DWORD kRegistrySchemaVersion = 1;
    constexpr DWORD kServiceWaitMilliseconds = 15'000;
    constexpr std::size_t kSha256Length = 32;
    constexpr std::uint64_t kMaximumInfBytes = 4ULL * 1024ULL * 1024ULL;

    constexpr int kExitUsage = 2;
    constexpr int kExitInvalidPayload = 10;
    constexpr int kExitElevationRequired = 11;
    constexpr int kExitServiceFailure = 12;
    constexpr int kExitDriverFailure = 13;
    constexpr int kExitRegistryFailure = 14;
    constexpr int kExitVerificationFailure = 15;
    constexpr int kExitRebootRequired = 3010;

    struct HandleCloser
    {
        void operator()(HANDLE handle) const noexcept
        {
            if (handle != nullptr && handle != INVALID_HANDLE_VALUE)
            {
                CloseHandle(handle);
            }
        }
    };

    struct ServiceHandleCloser
    {
        void operator()(SC_HANDLE handle) const noexcept
        {
            if (handle != nullptr)
            {
                CloseServiceHandle(handle);
            }
        }
    };

    struct RegistryHandleCloser
    {
        void operator()(HKEY key) const noexcept
        {
            if (key != nullptr)
            {
                RegCloseKey(key);
            }
        }
    };

    struct AlgorithmCloser
    {
        void operator()(BCRYPT_ALG_HANDLE handle) const noexcept
        {
            if (handle != nullptr)
            {
                BCryptCloseAlgorithmProvider(handle, 0);
            }
        }
    };

    struct HashCloser
    {
        void operator()(BCRYPT_HASH_HANDLE handle) const noexcept
        {
            if (handle != nullptr)
            {
                BCryptDestroyHash(handle);
            }
        }
    };

    using UniqueHandle = std::unique_ptr<std::remove_pointer_t<HANDLE>, HandleCloser>;
    using UniqueServiceHandle = std::unique_ptr<std::remove_pointer_t<SC_HANDLE>, ServiceHandleCloser>;
    using UniqueRegistryHandle = std::unique_ptr<std::remove_pointer_t<HKEY>, RegistryHandleCloser>;
    using UniqueAlgorithm = std::unique_ptr<std::remove_pointer_t<BCRYPT_ALG_HANDLE>, AlgorithmCloser>;
    using UniqueHash = std::unique_ptr<std::remove_pointer_t<BCRYPT_HASH_HANDLE>, HashCloser>;

    DWORD CryptoError(NTSTATUS status)
    {
        return status < 0 ? static_cast<DWORD>(status) : ERROR_SUCCESS;
    }

    struct DriverRecord
    {
        std::wstring publishedName;
        std::array<std::uint8_t, kSha256Length> infHash{};
    };

    struct DriverInstallResult
    {
        std::wstring publishedName;
        std::wstring publishedPath;
        std::array<std::uint8_t, kSha256Length> infHash{};
        bool newlyCopied = false;
        bool rebootRequired = false;
    };

    std::wstring JoinPath(const std::wstring& base, const std::wstring& child)
    {
        if (base.empty() || base.back() == L'\\')
        {
            return base + child;
        }
        return base + L"\\" + child;
    }

    std::wstring ToLower(std::wstring value)
    {
        std::transform(value.begin(), value.end(), value.begin(), [](wchar_t character) {
            return static_cast<wchar_t>(towlower(character));
        });
        return value;
    }

    std::string WideToUtf8(const std::wstring& value)
    {
        if (value.empty())
        {
            return {};
        }
        const int length = WideCharToMultiByte(
            CP_UTF8, WC_ERR_INVALID_CHARS, value.c_str(), static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
        if (length <= 0)
        {
            return "invalid UTF-16 detail";
        }
        std::string result(static_cast<std::size_t>(length), '\0');
        WideCharToMultiByte(
            CP_UTF8,
            WC_ERR_INVALID_CHARS,
            value.c_str(),
            static_cast<int>(value.size()),
            result.data(),
            length,
            nullptr,
            nullptr);
        return result;
    }

    std::string JsonEscape(const std::string& value)
    {
        static constexpr char digits[] = "0123456789abcdef";
        std::string escaped;
        escaped.reserve(value.size() + 16);
        for (const unsigned char character : value)
        {
            switch (character)
            {
            case '\"':
                escaped += "\\\"";
                break;
            case '\\':
                escaped += "\\\\";
                break;
            case '\b':
                escaped += "\\b";
                break;
            case '\f':
                escaped += "\\f";
                break;
            case '\n':
                escaped += "\\n";
                break;
            case '\r':
                escaped += "\\r";
                break;
            case '\t':
                escaped += "\\t";
                break;
            default:
                if (character < 0x20)
                {
                    escaped += "\\u00";
                    escaped.push_back(digits[(character >> 4U) & 0x0fU]);
                    escaped.push_back(digits[character & 0x0fU]);
                }
                else
                {
                    escaped.push_back(static_cast<char>(character));
                }
                break;
            }
        }
        return escaped;
    }

    std::wstring Win32Message(DWORD error)
    {
        wchar_t* buffer = nullptr;
        const DWORD length = FormatMessageW(
            FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
            nullptr,
            error,
            0,
            reinterpret_cast<LPWSTR>(&buffer),
            0,
            nullptr);
        std::wstring result = length == 0 || buffer == nullptr ? L"Windows error" : std::wstring(buffer, length);
        if (buffer != nullptr)
        {
            LocalFree(buffer);
        }
        while (!result.empty() && (result.back() == L'\r' || result.back() == L'\n' || result.back() == L' '))
        {
            result.pop_back();
        }
        return result;
    }

    int PrintReport(
        const char* operation,
        bool success,
        const char* errorCode,
        DWORD win32,
        const std::wstring& detail,
        bool rebootRequired,
        int exitCode,
        const std::string& extraFields = {})
    {
        std::cout << "{\"schemaVersion\":1,\"operation\":\"" << operation << "\",\"success\":"
                  << (success ? "true" : "false") << ",\"rebootRequired\":"
                  << (rebootRequired ? "true" : "false") << ",\"errorCode\":\"" << errorCode
                  << "\",\"win32\":" << win32 << ",\"detail\":\"" << JsonEscape(WideToUtf8(detail)) << "\"";
        if (!extraFields.empty())
        {
            std::cout << ',' << extraFields;
        }
        std::cout << "}\n";
        return exitCode;
    }

    int PrintWin32Failure(const char* operation, const char* errorCode, DWORD error, int exitCode)
    {
        return PrintReport(operation, false, errorCode, error, Win32Message(error), false, exitCode);
    }

    bool IsElevated(DWORD& error)
    {
        HANDLE rawToken = nullptr;
        if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &rawToken))
        {
            error = GetLastError();
            return false;
        }
        UniqueHandle token(rawToken);
        TOKEN_ELEVATION elevation{};
        DWORD returned = 0;
        if (!GetTokenInformation(token.get(), TokenElevation, &elevation, sizeof(elevation), &returned))
        {
            error = GetLastError();
            return false;
        }
        error = ERROR_SUCCESS;
        return elevation.TokenIsElevated != 0;
    }

    bool GetModuleDirectory(std::wstring& directory, DWORD& error)
    {
        std::vector<wchar_t> buffer(512);
        for (;;)
        {
            const DWORD length = GetModuleFileNameW(nullptr, buffer.data(), static_cast<DWORD>(buffer.size()));
            if (length == 0)
            {
                error = GetLastError();
                return false;
            }
            if (length < buffer.size() - 1)
            {
                std::wstring path(buffer.data(), length);
                const std::size_t separator = path.find_last_of(L"\\/");
                if (separator == std::wstring::npos)
                {
                    error = ERROR_BAD_PATHNAME;
                    return false;
                }
                directory = path.substr(0, separator);
                error = ERROR_SUCCESS;
                return true;
            }
            if (buffer.size() >= 32'768)
            {
                error = ERROR_FILENAME_EXCED_RANGE;
                return false;
            }
            buffer.resize(buffer.size() * 2);
        }
    }

    bool IsRegularPayloadFile(const std::wstring& path, DWORD& error)
    {
        WIN32_FILE_ATTRIBUTE_DATA data{};
        if (!GetFileAttributesExW(path.c_str(), GetFileExInfoStandard, &data))
        {
            error = GetLastError();
            return false;
        }
        if ((data.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT)) != 0)
        {
            error = ERROR_INVALID_DATA;
            return false;
        }
        if (data.nFileSizeHigh == 0 && data.nFileSizeLow == 0)
        {
            error = ERROR_FILE_INVALID;
            return false;
        }
        error = ERROR_SUCCESS;
        return true;
    }

    struct NativeOsVersion
    {
        ULONG size;
        ULONG major;
        ULONG minor;
        ULONG build;
        ULONG platform;
        wchar_t servicePack[128];
    };

    bool ValidateSupportedHost(DWORD& error)
    {
        using RtlGetVersionFunction = LONG(WINAPI*)(NativeOsVersion*);
        const HMODULE ntdll = GetModuleHandleW(L"ntdll.dll");
        if (ntdll == nullptr)
        {
            error = GetLastError();
            return false;
        }
        const auto rtlGetVersion = reinterpret_cast<RtlGetVersionFunction>(
            GetProcAddress(ntdll, "RtlGetVersion"));
        if (rtlGetVersion == nullptr)
        {
            error = ERROR_PROC_NOT_FOUND;
            return false;
        }
        NativeOsVersion version{};
        version.size = sizeof(version);
        if (rtlGetVersion(&version) < 0)
        {
            error = ERROR_OLD_WIN_VERSION;
            return false;
        }
        if (version.major < 10 || (version.major == 10 && version.build < 22'000))
        {
            error = ERROR_OLD_WIN_VERSION;
            return false;
        }

        USHORT processMachine = IMAGE_FILE_MACHINE_UNKNOWN;
        USHORT nativeMachine = IMAGE_FILE_MACHINE_UNKNOWN;
        if (!IsWow64Process2(GetCurrentProcess(), &processMachine, &nativeMachine))
        {
            error = GetLastError();
            return false;
        }
        if (nativeMachine != IMAGE_FILE_MACHINE_AMD64)
        {
            error = ERROR_NOT_SUPPORTED;
            return false;
        }
        error = ERROR_SUCCESS;
        return true;
    }

    bool ReadFileBytes(const std::wstring& path, std::vector<std::uint8_t>& bytes, DWORD& error)
    {
        if (!IsRegularPayloadFile(path, error))
        {
            return false;
        }
        UniqueHandle file(CreateFileW(
            path.c_str(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            nullptr,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            nullptr));
        if (file.get() == INVALID_HANDLE_VALUE)
        {
            error = GetLastError();
            return false;
        }
        LARGE_INTEGER size{};
        if (!GetFileSizeEx(file.get(), &size))
        {
            error = GetLastError();
            return false;
        }
        if (size.QuadPart <= 0 || static_cast<std::uint64_t>(size.QuadPart) > kMaximumInfBytes)
        {
            error = ERROR_FILE_TOO_LARGE;
            return false;
        }
        bytes.resize(static_cast<std::size_t>(size.QuadPart));
        std::size_t offset = 0;
        while (offset < bytes.size())
        {
            const DWORD request = static_cast<DWORD>(std::min<std::size_t>(bytes.size() - offset, 64U * 1024U));
            DWORD received = 0;
            if (!ReadFile(file.get(), bytes.data() + offset, request, &received, nullptr))
            {
                error = GetLastError();
                return false;
            }
            if (received == 0)
            {
                error = ERROR_HANDLE_EOF;
                return false;
            }
            offset += received;
        }
        error = ERROR_SUCCESS;
        return true;
    }

    bool ComputeSha256(
        const std::vector<std::uint8_t>& bytes,
        std::array<std::uint8_t, kSha256Length>& digest,
        DWORD& error)
    {
        BCRYPT_ALG_HANDLE rawAlgorithm = nullptr;
        NTSTATUS status = BCryptOpenAlgorithmProvider(&rawAlgorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0);
        if (status < 0)
        {
            error = CryptoError(status);
            return false;
        }
        UniqueAlgorithm algorithm(rawAlgorithm);

        DWORD objectLength = 0;
        DWORD returned = 0;
        status = BCryptGetProperty(
            algorithm.get(),
            BCRYPT_OBJECT_LENGTH,
            reinterpret_cast<PUCHAR>(&objectLength),
            sizeof(objectLength),
            &returned,
            0);
        if (status < 0 || objectLength == 0)
        {
            error = status < 0 ? CryptoError(status) : ERROR_INVALID_DATA;
            return false;
        }
        DWORD hashLength = 0;
        status = BCryptGetProperty(
            algorithm.get(),
            BCRYPT_HASH_LENGTH,
            reinterpret_cast<PUCHAR>(&hashLength),
            sizeof(hashLength),
            &returned,
            0);
        if (status < 0 || hashLength != digest.size())
        {
            error = status < 0 ? CryptoError(status) : ERROR_INVALID_DATA;
            return false;
        }

        std::vector<std::uint8_t> object(objectLength);
        BCRYPT_HASH_HANDLE rawHash = nullptr;
        status = BCryptCreateHash(algorithm.get(), &rawHash, object.data(), objectLength, nullptr, 0, 0);
        if (status < 0)
        {
            error = CryptoError(status);
            return false;
        }
        UniqueHash hash(rawHash);
        if (bytes.size() > ULONG_MAX)
        {
            error = ERROR_FILE_TOO_LARGE;
            return false;
        }
        status = BCryptHashData(hash.get(), const_cast<PUCHAR>(bytes.data()), static_cast<ULONG>(bytes.size()), 0);
        if (status < 0)
        {
            error = CryptoError(status);
            return false;
        }
        status = BCryptFinishHash(hash.get(), digest.data(), static_cast<ULONG>(digest.size()), 0);
        if (status < 0)
        {
            error = CryptoError(status);
            return false;
        }
        error = ERROR_SUCCESS;
        return true;
    }

    bool HashFile(
        const std::wstring& path,
        std::array<std::uint8_t, kSha256Length>& digest,
        DWORD& error)
    {
        std::vector<std::uint8_t> bytes;
        return ReadFileBytes(path, bytes, error) && ComputeSha256(bytes, digest, error);
    }

    bool HasLadoFlowInfIdentity(const std::vector<std::uint8_t>& bytes)
    {
        std::string compact;
        compact.reserve(bytes.size());
        for (const std::uint8_t byte : bytes)
        {
            if (byte == 0)
            {
                return false;
            }
            const unsigned char character = byte;
            if (!std::isspace(character))
            {
                compact.push_back(static_cast<char>(std::tolower(character)));
            }
        }
        constexpr const char* markers[] = {
            "classguid={4d36e968-e325-11ce-bfc1-08002be10318}",
            "provider=%manufacturername%",
            "manufacturername=\"ladoflowproject\"",
            "catalogfile=ladoflowidd.cat",
            "ladoflowvirtualdisplay",
            "umdfservice=ladoflowidd,ladoflowidd_service",
        };
        return std::all_of(std::begin(markers), std::end(markers), [&compact](const char* marker) {
            return compact.find(marker) != std::string::npos;
        });
    }

    bool ValidateInfFile(const std::wstring& path, DWORD& error)
    {
        std::vector<std::uint8_t> bytes;
        if (!ReadFileBytes(path, bytes, error))
        {
            return false;
        }
        if (!HasLadoFlowInfIdentity(bytes))
        {
            error = ERROR_INVALID_DATA;
            return false;
        }
        error = ERROR_SUCCESS;
        return true;
    }

    bool ValidatePayload(const std::wstring& root, DWORD& error)
    {
        const std::wstring files[] = {
            JoinPath(root, kServiceBinaryName),
            JoinPath(root, kControllerBinaryName),
            JoinPath(root, kDriverInfRelativePath),
            JoinPath(root, kDriverDllRelativePath),
            JoinPath(root, kDriverCatalogRelativePath),
        };
        for (const std::wstring& file : files)
        {
            if (!IsRegularPayloadFile(file, error))
            {
                return false;
            }
        }
        return ValidateInfFile(JoinPath(root, kDriverInfRelativePath), error);
    }

    bool IsStrictPublishedInfName(const std::wstring& value)
    {
        const std::wstring lower = ToLower(value);
        if (lower.size() < 8 || lower.compare(0, 3, L"oem") != 0 || lower.compare(lower.size() - 4, 4, L".inf") != 0)
        {
            return false;
        }
        return std::all_of(lower.begin() + 3, lower.end() - 4, [](wchar_t character) {
            return character >= L'0' && character <= L'9';
        });
    }

    std::wstring BaseName(const std::wstring& path)
    {
        const std::size_t separator = path.find_last_of(L"\\/");
        return separator == std::wstring::npos ? path : path.substr(separator + 1);
    }

    bool ParseServiceCommand(const std::wstring& command, std::wstring& executable)
    {
        const std::size_t first = command.find_first_not_of(L" \t");
        if (first == std::wstring::npos)
        {
            return false;
        }
        std::size_t argumentStart = std::wstring::npos;
        if (command[first] == L'\"')
        {
            const std::size_t closingQuote = command.find(L'\"', first + 1);
            if (closingQuote == std::wstring::npos || closingQuote == first + 1)
            {
                return false;
            }
            executable = command.substr(first + 1, closingQuote - first - 1);
            argumentStart = closingQuote + 1;
        }
        else
        {
            const std::size_t separator = command.find_first_of(L" \t", first);
            executable = command.substr(first, separator == std::wstring::npos ? separator : separator - first);
            argumentStart = separator;
        }
        if (argumentStart == std::wstring::npos)
        {
            return false;
        }
        const std::size_t argument = command.find_first_not_of(L" \t", argumentStart);
        if (argument == std::wstring::npos)
        {
            return false;
        }
        const std::size_t trailing = command.find_last_not_of(L" \t");
        return ToLower(command.substr(argument, trailing - argument + 1)) == L"service";
    }

    bool GetInstalledInfPath(const std::wstring& publishedName, std::wstring& path, DWORD& error)
    {
        if (!IsStrictPublishedInfName(publishedName))
        {
            error = ERROR_INVALID_NAME;
            return false;
        }
        std::array<wchar_t, MAX_PATH + 1> windows{};
        const UINT length = GetWindowsDirectoryW(windows.data(), static_cast<UINT>(windows.size()));
        if (length == 0 || length >= windows.size())
        {
            error = length == 0 ? GetLastError() : ERROR_INSUFFICIENT_BUFFER;
            return false;
        }
        path = JoinPath(JoinPath(windows.data(), L"INF"), ToLower(publishedName));
        error = ERROR_SUCCESS;
        return true;
    }

    bool ValidateRecordedInf(const DriverRecord& record, std::wstring& path, DWORD& error)
    {
        if (!GetInstalledInfPath(record.publishedName, path, error))
        {
            return false;
        }
        if (!ValidateInfFile(path, error))
        {
            return false;
        }
        std::array<std::uint8_t, kSha256Length> actual{};
        if (!HashFile(path, actual, error))
        {
            return false;
        }
        if (actual != record.infHash)
        {
            error = ERROR_DATA_CHECKSUM_ERROR;
            return false;
        }
        error = ERROR_SUCCESS;
        return true;
    }

    bool RecordDriverPackage(
        const std::wstring& installRoot,
        const DriverInstallResult& driver,
        DWORD& error)
    {
        if (!IsStrictPublishedInfName(driver.publishedName))
        {
            error = ERROR_INVALID_NAME;
            return false;
        }
        HKEY rawRoot = nullptr;
        LONG result = RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            kRegistryPath,
            0,
            nullptr,
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
            nullptr,
            &rawRoot,
            nullptr);
        if (result != ERROR_SUCCESS)
        {
            error = static_cast<DWORD>(result);
            return false;
        }
        UniqueRegistryHandle root(rawRoot);
        result = RegSetValueExW(
            root.get(),
            L"SchemaVersion",
            0,
            REG_DWORD,
            reinterpret_cast<const BYTE*>(&kRegistrySchemaVersion),
            sizeof(kRegistrySchemaVersion));
        if (result == ERROR_SUCCESS)
        {
            result = RegSetValueExW(
                root.get(),
                L"InstallRoot",
                0,
                REG_SZ,
                reinterpret_cast<const BYTE*>(installRoot.c_str()),
                static_cast<DWORD>((installRoot.size() + 1) * sizeof(wchar_t)));
        }
        if (result != ERROR_SUCCESS)
        {
            error = static_cast<DWORD>(result);
            return false;
        }

        HKEY rawDrivers = nullptr;
        result = RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            kDriversRegistryPath,
            0,
            nullptr,
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
            nullptr,
            &rawDrivers,
            nullptr);
        if (result != ERROR_SUCCESS)
        {
            error = static_cast<DWORD>(result);
            return false;
        }
        UniqueRegistryHandle drivers(rawDrivers);
        HKEY rawPackage = nullptr;
        DWORD disposition = 0;
        const std::wstring packageName = ToLower(driver.publishedName);
        result = RegCreateKeyExW(
            drivers.get(),
            packageName.c_str(),
            0,
            nullptr,
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE | KEY_WOW64_64KEY,
            nullptr,
            &rawPackage,
            &disposition);
        if (result != ERROR_SUCCESS)
        {
            error = static_cast<DWORD>(result);
            return false;
        }
        UniqueRegistryHandle package(rawPackage);
        result = RegSetValueExW(
            package.get(),
            L"InfSha256",
            0,
            REG_BINARY,
            driver.infHash.data(),
            static_cast<DWORD>(driver.infHash.size()));
        if (result != ERROR_SUCCESS)
        {
            package.reset();
            if (disposition == REG_CREATED_NEW_KEY)
            {
                RegDeleteTreeW(drivers.get(), packageName.c_str());
            }
            error = static_cast<DWORD>(result);
            return false;
        }
        error = ERROR_SUCCESS;
        return true;
    }

    bool ReadDriverRecords(std::vector<DriverRecord>& records, DWORD& error)
    {
        HKEY rawDrivers = nullptr;
        LONG result = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            kDriversRegistryPath,
            0,
            KEY_READ | KEY_WOW64_64KEY,
            &rawDrivers);
        if (result == ERROR_FILE_NOT_FOUND)
        {
            error = ERROR_SUCCESS;
            return true;
        }
        if (result != ERROR_SUCCESS)
        {
            error = static_cast<DWORD>(result);
            return false;
        }
        UniqueRegistryHandle drivers(rawDrivers);
        for (DWORD index = 0;; ++index)
        {
            std::array<wchar_t, 256> name{};
            DWORD length = static_cast<DWORD>(name.size());
            result = RegEnumKeyExW(drivers.get(), index, name.data(), &length, nullptr, nullptr, nullptr, nullptr);
            if (result == ERROR_NO_MORE_ITEMS)
            {
                break;
            }
            if (result != ERROR_SUCCESS)
            {
                error = static_cast<DWORD>(result);
                return false;
            }
            DriverRecord record;
            record.publishedName.assign(name.data(), length);
            if (!IsStrictPublishedInfName(record.publishedName))
            {
                error = ERROR_INVALID_DATA;
                return false;
            }
            HKEY rawPackage = nullptr;
            result = RegOpenKeyExW(
                drivers.get(), record.publishedName.c_str(), 0, KEY_QUERY_VALUE | KEY_WOW64_64KEY, &rawPackage);
            if (result != ERROR_SUCCESS)
            {
                error = static_cast<DWORD>(result);
                return false;
            }
            UniqueRegistryHandle package(rawPackage);
            DWORD type = 0;
            DWORD size = static_cast<DWORD>(record.infHash.size());
            result = RegQueryValueExW(
                package.get(), L"InfSha256", nullptr, &type, record.infHash.data(), &size);
            if (result != ERROR_SUCCESS || type != REG_BINARY || size != record.infHash.size())
            {
                error = result == ERROR_SUCCESS ? ERROR_INVALID_DATA : static_cast<DWORD>(result);
                return false;
            }
            records.push_back(record);
        }
        error = ERROR_SUCCESS;
        return true;
    }

    bool IsOwnedServiceConfiguration(SC_HANDLE service, DWORD& error)
    {
        DWORD required = 0;
        QueryServiceConfigW(service, nullptr, 0, &required);
        if (required == 0 || GetLastError() != ERROR_INSUFFICIENT_BUFFER)
        {
            error = GetLastError();
            return false;
        }
        std::vector<std::uint8_t> storage(required);
        auto* configuration = reinterpret_cast<QUERY_SERVICE_CONFIGW*>(storage.data());
        if (!QueryServiceConfigW(service, configuration, required, &required))
        {
            error = GetLastError();
            return false;
        }
        const std::wstring command = configuration->lpBinaryPathName == nullptr ? L"" : configuration->lpBinaryPathName;
        const std::wstring account = configuration->lpServiceStartName == nullptr ? L"" : ToLower(configuration->lpServiceStartName);
        const std::wstring display = configuration->lpDisplayName == nullptr ? L"" : configuration->lpDisplayName;
        std::wstring executable;
        const bool binaryMatches = ParseServiceCommand(command, executable) &&
                                   ToLower(BaseName(executable)) == ToLower(kServiceBinaryName);
        const bool accountMatches = account == L"localsystem" || account == L"nt authority\\system" || account == L".\\localsystem";
        if (!binaryMatches || !accountMatches || display != Ipc::kServiceDisplayName)
        {
            error = ERROR_SERVICE_EXISTS;
            return false;
        }
        error = ERROR_SUCCESS;
        return true;
    }

    bool WaitForServiceState(SC_HANDLE service, DWORD desiredState, DWORD timeoutMilliseconds, DWORD& error)
    {
        const ULONGLONG deadline = GetTickCount64() + timeoutMilliseconds;
        for (;;)
        {
            SERVICE_STATUS_PROCESS status{};
            DWORD returned = 0;
            if (!QueryServiceStatusEx(
                    service,
                    SC_STATUS_PROCESS_INFO,
                    reinterpret_cast<LPBYTE>(&status),
                    sizeof(status),
                    &returned))
            {
                error = GetLastError();
                return false;
            }
            if (status.dwCurrentState == desiredState)
            {
                error = ERROR_SUCCESS;
                return true;
            }
            if (desiredState == SERVICE_RUNNING && status.dwCurrentState == SERVICE_STOPPED)
            {
                error = status.dwWin32ExitCode == ERROR_SUCCESS ? ERROR_SERVICE_NOT_ACTIVE : status.dwWin32ExitCode;
                return false;
            }
            if (GetTickCount64() >= deadline)
            {
                error = ERROR_TIMEOUT;
                return false;
            }
            Sleep(100);
        }
    }

    bool StopServiceHandle(SC_HANDLE service, DWORD& error)
    {
        SERVICE_STATUS_PROCESS status{};
        DWORD returned = 0;
        if (!QueryServiceStatusEx(
                service,
                SC_STATUS_PROCESS_INFO,
                reinterpret_cast<LPBYTE>(&status),
                sizeof(status),
                &returned))
        {
            error = GetLastError();
            return false;
        }
        if (status.dwCurrentState == SERVICE_STOPPED)
        {
            error = ERROR_SUCCESS;
            return true;
        }
        if (status.dwCurrentState != SERVICE_STOP_PENDING)
        {
            SERVICE_STATUS ignored{};
            if (!ControlService(service, SERVICE_CONTROL_STOP, &ignored))
            {
                const DWORD stopError = GetLastError();
                if (stopError != ERROR_SERVICE_NOT_ACTIVE)
                {
                    error = stopError;
                    return false;
                }
            }
        }
        return WaitForServiceState(service, SERVICE_STOPPED, kServiceWaitMilliseconds, error);
    }

    bool OpenServiceManager(UniqueServiceHandle& manager, DWORD access, DWORD& error)
    {
        manager.reset(OpenSCManagerW(nullptr, nullptr, access));
        if (!manager)
        {
            error = GetLastError();
            return false;
        }
        error = ERROR_SUCCESS;
        return true;
    }

    bool PrepareExistingService(DWORD& error)
    {
        UniqueServiceHandle manager;
        if (!OpenServiceManager(manager, SC_MANAGER_CONNECT, error))
        {
            return false;
        }
        UniqueServiceHandle service(OpenServiceW(
            manager.get(),
            Ipc::kServiceName,
            SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS | SERVICE_STOP));
        if (!service)
        {
            error = GetLastError();
            if (error == ERROR_SERVICE_DOES_NOT_EXIST)
            {
                error = ERROR_SUCCESS;
                return true;
            }
            return false;
        }
        return IsOwnedServiceConfiguration(service.get(), error) && StopServiceHandle(service.get(), error);
    }

    std::wstring QuoteServiceCommand(const std::wstring& servicePath)
    {
        return L"\"" + servicePath + L"\" service";
    }

    bool ConfigureService(SC_HANDLE service, DWORD& error)
    {
        SERVICE_DESCRIPTIONW description{};
        description.lpDescription = const_cast<LPWSTR>(
            L"Owns the LadoFlow software device and virtual display adapter lifecycle.");
        if (!ChangeServiceConfig2W(service, SERVICE_CONFIG_DESCRIPTION, &description))
        {
            error = GetLastError();
            return false;
        }

        SERVICE_DELAYED_AUTO_START_INFO delayed{};
        delayed.fDelayedAutostart = TRUE;
        if (!ChangeServiceConfig2W(service, SERVICE_CONFIG_DELAYED_AUTO_START_INFO, &delayed))
        {
            error = GetLastError();
            return false;
        }

        SC_ACTION actions[] = {
            {SC_ACTION_RESTART, 2'000},
            {SC_ACTION_RESTART, 10'000},
            {SC_ACTION_NONE, 0},
        };
        SERVICE_FAILURE_ACTIONSW failureActions{};
        failureActions.dwResetPeriod = 24U * 60U * 60U;
        failureActions.cActions = static_cast<DWORD>(std::size(actions));
        failureActions.lpsaActions = actions;
        if (!ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS, &failureActions))
        {
            error = GetLastError();
            return false;
        }

        SERVICE_FAILURE_ACTIONS_FLAG failureFlag{};
        failureFlag.fFailureActionsOnNonCrashFailures = TRUE;
        if (!ChangeServiceConfig2W(service, SERVICE_CONFIG_FAILURE_ACTIONS_FLAG, &failureFlag))
        {
            error = GetLastError();
            return false;
        }
        error = ERROR_SUCCESS;
        return true;
    }

    bool InstallOrUpdateService(const std::wstring& root, bool& created, DWORD& error)
    {
        created = false;
        const std::wstring servicePath = JoinPath(root, kServiceBinaryName);
        const std::wstring command = QuoteServiceCommand(servicePath);
        UniqueServiceHandle manager;
        if (!OpenServiceManager(manager, SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE, error))
        {
            return false;
        }
        constexpr DWORD access = SERVICE_CHANGE_CONFIG | SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS |
                                 SERVICE_START | SERVICE_STOP | DELETE;
        UniqueServiceHandle service(OpenServiceW(manager.get(), Ipc::kServiceName, access));
        if (!service)
        {
            const DWORD openError = GetLastError();
            if (openError != ERROR_SERVICE_DOES_NOT_EXIST)
            {
                error = openError;
                return false;
            }
            service.reset(CreateServiceW(
                manager.get(),
                Ipc::kServiceName,
                Ipc::kServiceDisplayName,
                access,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_AUTO_START,
                SERVICE_ERROR_NORMAL,
                command.c_str(),
                nullptr,
                nullptr,
                nullptr,
                nullptr,
                nullptr));
            if (!service)
            {
                error = GetLastError();
                return false;
            }
            created = true;
        }
        else
        {
            if (!IsOwnedServiceConfiguration(service.get(), error) || !StopServiceHandle(service.get(), error))
            {
                return false;
            }
            if (!ChangeServiceConfigW(
                    service.get(),
                    SERVICE_WIN32_OWN_PROCESS,
                    SERVICE_AUTO_START,
                    SERVICE_ERROR_NORMAL,
                    command.c_str(),
                    nullptr,
                    nullptr,
                    nullptr,
                    nullptr,
                    nullptr,
                    Ipc::kServiceDisplayName))
            {
                error = GetLastError();
                return false;
            }
        }
        if (!ConfigureService(service.get(), error))
        {
            return false;
        }
        if (!StartServiceW(service.get(), 0, nullptr))
        {
            const DWORD startError = GetLastError();
            if (startError != ERROR_SERVICE_ALREADY_RUNNING)
            {
                error = startError;
                return false;
            }
        }
        return WaitForServiceState(service.get(), SERVICE_RUNNING, kServiceWaitMilliseconds, error);
    }

    bool DeleteOwnedService(DWORD& error)
    {
        UniqueServiceHandle manager;
        if (!OpenServiceManager(manager, SC_MANAGER_CONNECT, error))
        {
            return false;
        }
        UniqueServiceHandle service(OpenServiceW(
            manager.get(),
            Ipc::kServiceName,
            SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS | SERVICE_STOP | DELETE));
        if (!service)
        {
            error = GetLastError();
            if (error == ERROR_SERVICE_DOES_NOT_EXIST)
            {
                error = ERROR_SUCCESS;
                return true;
            }
            return false;
        }
        if (!IsOwnedServiceConfiguration(service.get(), error) || !StopServiceHandle(service.get(), error))
        {
            return false;
        }
        if (!DeleteService(service.get()))
        {
            error = GetLastError();
            if (error != ERROR_SERVICE_MARKED_FOR_DELETE)
            {
                return false;
            }
        }
        error = ERROR_SUCCESS;
        return true;
    }

    bool RollBackNewService(DWORD& error)
    {
        return DeleteOwnedService(error);
    }

    bool InstallDriverPackage(const std::wstring& sourceInf, DriverInstallResult& result, DWORD& error)
    {
        std::array<wchar_t, MAX_PATH> destination{};
        DWORD required = 0;
        wchar_t* component = nullptr;
        if (SetupCopyOEMInfW(
                sourceInf.c_str(),
                nullptr,
                SPOST_PATH,
                SP_COPY_NOOVERWRITE,
                destination.data(),
                static_cast<DWORD>(destination.size()),
                &required,
                &component))
        {
            result.newlyCopied = true;
        }
        else
        {
            error = GetLastError();
            if (error != ERROR_FILE_EXISTS)
            {
                return false;
            }
            result.newlyCopied = false;
        }
        result.publishedPath.assign(destination.data());
        result.publishedName = component == nullptr ? BaseName(result.publishedPath) : component;
        if (!IsStrictPublishedInfName(result.publishedName) || !ValidateInfFile(result.publishedPath, error))
        {
            return false;
        }
        std::array<std::uint8_t, kSha256Length> sourceHash{};
        if (!HashFile(sourceInf, sourceHash, error) || !HashFile(result.publishedPath, result.infHash, error))
        {
            return false;
        }
        if (sourceHash != result.infHash)
        {
            error = ERROR_DATA_CHECKSUM_ERROR;
            return false;
        }
        BOOL reboot = FALSE;
        if (!DiInstallDriverW(nullptr, sourceInf.c_str(), 0, &reboot))
        {
            error = GetLastError();
            return false;
        }
        result.rebootRequired = reboot != FALSE;
        error = ERROR_SUCCESS;
        return true;
    }

    bool RemoveDriverPackage(const std::wstring& installedInf, bool& rebootRequired, DWORD& error)
    {
        BOOL reboot = FALSE;
        if (!DiUninstallDriverW(nullptr, installedInf.c_str(), 0, &reboot))
        {
            error = GetLastError();
            if (error == ERROR_FILE_NOT_FOUND)
            {
                error = ERROR_SUCCESS;
                return true;
            }
            return false;
        }
        rebootRequired = rebootRequired || reboot != FALSE;
        error = ERROR_SUCCESS;
        return true;
    }

    void RollBackNewDriver(const DriverInstallResult& driver)
    {
        if (!driver.newlyCopied || driver.publishedPath.empty())
        {
            return;
        }
        BOOL ignoredReboot = FALSE;
        if (!DiUninstallDriverW(nullptr, driver.publishedPath.c_str(), 0, &ignoredReboot))
        {
            SetupUninstallOEMInfW(driver.publishedName.c_str(), 0, nullptr);
        }
    }

    bool QueryServiceInstalledAndRunning(bool& installed, bool& running, DWORD& error)
    {
        installed = false;
        running = false;
        UniqueServiceHandle manager;
        if (!OpenServiceManager(manager, SC_MANAGER_CONNECT, error))
        {
            return false;
        }
        UniqueServiceHandle service(OpenServiceW(
            manager.get(), Ipc::kServiceName, SERVICE_QUERY_CONFIG | SERVICE_QUERY_STATUS));
        if (!service)
        {
            error = GetLastError();
            if (error == ERROR_SERVICE_DOES_NOT_EXIST)
            {
                error = ERROR_SUCCESS;
                return true;
            }
            return false;
        }
        if (!IsOwnedServiceConfiguration(service.get(), error))
        {
            return false;
        }
        SERVICE_STATUS_PROCESS status{};
        DWORD returned = 0;
        if (!QueryServiceStatusEx(
                service.get(),
                SC_STATUS_PROCESS_INFO,
                reinterpret_cast<LPBYTE>(&status),
                sizeof(status),
                &returned))
        {
            error = GetLastError();
            return false;
        }
        installed = true;
        running = status.dwCurrentState == SERVICE_RUNNING;
        error = ERROR_SUCCESS;
        return true;
    }

    int RunSelfTest()
    {
        const std::string validInf = R"(
            [Version]
            ClassGUID={4D36E968-E325-11CE-BFC1-08002BE10318}
            Provider=%ManufacturerName%
            CatalogFile=LadoFlowIdd.cat
            [Strings]
            ManufacturerName="LadoFlow Project"
            [Models]
            Device=LadoFlowVirtualDisplay
            UmdfService=LadoFlowIdd,LadoFlowIdd_Service
        )";
        const std::vector<std::uint8_t> validBytes(validInf.begin(), validInf.end());
        const bool passed = IsStrictPublishedInfName(L"oem0.inf") &&
                            IsStrictPublishedInfName(L"OEM12345.INF") &&
                            !IsStrictPublishedInfName(L"oem.inf") &&
                            !IsStrictPublishedInfName(L"..\\oem1.inf") &&
                            !IsStrictPublishedInfName(L"oem1.inf.bak") &&
                            HasLadoFlowInfIdentity(validBytes) &&
                            !HasLadoFlowInfIdentity(std::vector<std::uint8_t>{'b', 'a', 'd'}) &&
                            JsonEscape("a\n\"b\\c") == "a\\n\\\"b\\\\c" &&
                            QuoteServiceCommand(L"C:\\Program Files\\LadoFlow\\service.exe") ==
                                L"\"C:\\Program Files\\LadoFlow\\service.exe\" service";
        std::wstring parsedService;
        const bool serviceCommandPassed =
            ParseServiceCommand(L"\"C:\\Program Files\\LadoFlow\\LadoFlowDisplayService.exe\" service", parsedService) &&
            parsedService == L"C:\\Program Files\\LadoFlow\\LadoFlowDisplayService.exe" &&
            !ParseServiceCommand(L"C:\\BadLadoFlowDisplayService.exe service extra", parsedService) &&
            !ParseServiceCommand(L"\"C:\\Bad.exe\"", parsedService);
        if (!passed || !serviceCommandPassed)
        {
            return PrintReport(
                "self-test", false, "assertion_failed", ERROR_INVALID_DATA, L"A setup helper invariant failed.", false, 1);
        }
        return PrintReport("self-test", true, "none", ERROR_SUCCESS, L"All setup helper invariants passed.", false, 0);
    }

    int RunPlanInstall(const std::wstring& root)
    {
        DWORD error = ERROR_SUCCESS;
        if (!ValidateSupportedHost(error))
        {
            return PrintWin32Failure("plan-install", "unsupported_host", error, kExitInvalidPayload);
        }
        if (!ValidatePayload(root, error))
        {
            return PrintWin32Failure("plan-install", "invalid_payload", error, kExitInvalidPayload);
        }
        std::array<std::uint8_t, kSha256Length> digest{};
        if (!HashFile(JoinPath(root, kDriverInfRelativePath), digest, error))
        {
            return PrintWin32Failure("plan-install", "hash_failed", error, kExitVerificationFailure);
        }
        return PrintReport(
            "plan-install",
            true,
            "none",
            ERROR_SUCCESS,
            L"Payload identity, non-reparse files, and driver INF hash are valid; no system state was changed.",
            false,
            0,
            "\"mutatesSystem\":false,\"payloadValid\":true");
    }

    int RunPlanUninstall()
    {
        DWORD error = ERROR_SUCCESS;
        std::vector<DriverRecord> records;
        if (!ReadDriverRecords(records, error))
        {
            return PrintWin32Failure("plan-uninstall", "registry_read_failed", error, kExitRegistryFailure);
        }
        std::size_t validDriverPackages = 0;
        std::size_t missingDriverPackages = 0;
        for (const DriverRecord& record : records)
        {
            std::wstring installedInf;
            if (ValidateRecordedInf(record, installedInf, error))
            {
                ++validDriverPackages;
                continue;
            }
            if (error == ERROR_FILE_NOT_FOUND || error == ERROR_PATH_NOT_FOUND)
            {
                ++missingDriverPackages;
                continue;
            }
            return PrintWin32Failure("plan-uninstall", "driver_identity_failed", error, kExitVerificationFailure);
        }
        bool serviceInstalled = false;
        bool serviceRunning = false;
        if (!QueryServiceInstalledAndRunning(serviceInstalled, serviceRunning, error))
        {
            return PrintWin32Failure("plan-uninstall", "service_query_failed", error, kExitServiceFailure);
        }
        return PrintReport(
            "plan-uninstall",
            true,
            "none",
            ERROR_SUCCESS,
            L"Uninstall state was inspected; no service or driver state was changed.",
            false,
            0,
            "\"mutatesSystem\":false,\"serviceInstalled\":" + std::string(serviceInstalled ? "true" : "false") +
                ",\"serviceRunning\":" + std::string(serviceRunning ? "true" : "false") +
                ",\"recordedDriverPackages\":" + std::to_string(records.size()) +
                ",\"validDriverPackages\":" + std::to_string(validDriverPackages) +
                ",\"missingDriverPackages\":" + std::to_string(missingDriverPackages));
    }

    int RunPrepareInstall()
    {
        DWORD error = ERROR_SUCCESS;
        if (!IsElevated(error))
        {
            return PrintWin32Failure(
                "prepare-install",
                error == ERROR_SUCCESS ? "elevation_required" : "token_query_failed",
                error == ERROR_SUCCESS ? ERROR_ELEVATION_REQUIRED : error,
                kExitElevationRequired);
        }
        if (!PrepareExistingService(error))
        {
            return PrintWin32Failure("prepare-install", "service_stop_failed", error, kExitServiceFailure);
        }
        return PrintReport(
            "prepare-install", true, "none", ERROR_SUCCESS, L"The owned service is stopped and ready for replacement.", false, 0);
    }

    int RunInstall(const std::wstring& root)
    {
        DWORD error = ERROR_SUCCESS;
        if (!IsElevated(error))
        {
            return PrintWin32Failure(
                "install",
                error == ERROR_SUCCESS ? "elevation_required" : "token_query_failed",
                error == ERROR_SUCCESS ? ERROR_ELEVATION_REQUIRED : error,
                kExitElevationRequired);
        }
        if (!ValidateSupportedHost(error))
        {
            return PrintWin32Failure("install", "unsupported_host", error, kExitInvalidPayload);
        }
        if (!ValidatePayload(root, error))
        {
            return PrintWin32Failure("install", "invalid_payload", error, kExitInvalidPayload);
        }

        DriverInstallResult driver;
        if (!InstallDriverPackage(JoinPath(root, kDriverInfRelativePath), driver, error))
        {
            RollBackNewDriver(driver);
            return PrintWin32Failure("install", "driver_install_failed", error, kExitDriverFailure);
        }

        bool serviceCreated = false;
        if (!InstallOrUpdateService(root, serviceCreated, error))
        {
            if (serviceCreated)
            {
                DWORD ignored = ERROR_SUCCESS;
                RollBackNewService(ignored);
            }
            RollBackNewDriver(driver);
            return PrintWin32Failure("install", "service_install_failed", error, kExitServiceFailure);
        }

        if (!RecordDriverPackage(root, driver, error))
        {
            DWORD ignored = ERROR_SUCCESS;
            PrepareExistingService(ignored);
            if (serviceCreated)
            {
                RollBackNewService(ignored);
            }
            RollBackNewDriver(driver);
            return PrintWin32Failure("install", "registry_write_failed", error, kExitRegistryFailure);
        }

        const int exitCode = driver.rebootRequired ? kExitRebootRequired : 0;
        return PrintReport(
            "install",
            true,
            "none",
            ERROR_SUCCESS,
            driver.rebootRequired ? L"Installation succeeded and Windows requested a restart."
                                   : L"The driver package and LocalSystem service were installed successfully.",
            driver.rebootRequired,
            exitCode,
            "\"driverPublishedName\":\"" + JsonEscape(WideToUtf8(driver.publishedName)) + "\"");
    }

    int RunUninstall()
    {
        DWORD error = ERROR_SUCCESS;
        if (!IsElevated(error))
        {
            return PrintWin32Failure(
                "uninstall",
                error == ERROR_SUCCESS ? "elevation_required" : "token_query_failed",
                error == ERROR_SUCCESS ? ERROR_ELEVATION_REQUIRED : error,
                kExitElevationRequired);
        }
        std::vector<DriverRecord> records;
        if (!ReadDriverRecords(records, error))
        {
            return PrintWin32Failure("uninstall", "registry_read_failed", error, kExitRegistryFailure);
        }
        for (const DriverRecord& record : records)
        {
            std::wstring installedInf;
            if (!ValidateRecordedInf(record, installedInf, error))
            {
                if (error == ERROR_FILE_NOT_FOUND || error == ERROR_PATH_NOT_FOUND)
                {
                    continue;
                }
                return PrintWin32Failure("uninstall", "driver_identity_failed", error, kExitVerificationFailure);
            }
        }
        if (!DeleteOwnedService(error))
        {
            return PrintWin32Failure("uninstall", "service_remove_failed", error, kExitServiceFailure);
        }

        bool rebootRequired = false;
        std::size_t removedDriverPackages = 0;
        for (const DriverRecord& record : records)
        {
            std::wstring installedInf;
            if (!GetInstalledInfPath(record.publishedName, installedInf, error))
            {
                return PrintWin32Failure("uninstall", "driver_path_failed", error, kExitVerificationFailure);
            }
            if (GetFileAttributesW(installedInf.c_str()) == INVALID_FILE_ATTRIBUTES && GetLastError() == ERROR_FILE_NOT_FOUND)
            {
                continue;
            }
            if (!RemoveDriverPackage(installedInf, rebootRequired, error))
            {
                return PrintWin32Failure("uninstall", "driver_remove_failed", error, kExitDriverFailure);
            }
            ++removedDriverPackages;
        }
        const LONG deleteResult = RegDeleteTreeW(HKEY_LOCAL_MACHINE, kRegistryPath);
        if (deleteResult != ERROR_SUCCESS && deleteResult != ERROR_FILE_NOT_FOUND)
        {
            return PrintWin32Failure(
                "uninstall", "registry_remove_failed", static_cast<DWORD>(deleteResult), kExitRegistryFailure);
        }
        const int exitCode = rebootRequired ? kExitRebootRequired : 0;
        return PrintReport(
            "uninstall",
            true,
            "none",
            ERROR_SUCCESS,
            rebootRequired ? L"Removal succeeded and Windows requested a restart."
                           : L"The owned service and recorded driver packages were removed successfully.",
            rebootRequired,
            exitCode,
            "\"removedDriverPackages\":" + std::to_string(removedDriverPackages));
    }
}

int wmain(int argc, wchar_t** argv)
{
    if (argc != 2)
    {
        return PrintReport(
            "usage",
            false,
            "invalid_arguments",
            ERROR_INVALID_PARAMETER,
            L"Usage: LadoFlowWindowsSetup <self-test|plan-install|plan-uninstall|prepare-install|install|uninstall>",
            false,
            kExitUsage);
    }
    const std::wstring command = argv[1];
    if (command == L"self-test")
    {
        return RunSelfTest();
    }
    if (command == L"plan-uninstall")
    {
        return RunPlanUninstall();
    }
    if (command == L"prepare-install")
    {
        return RunPrepareInstall();
    }
    if (command == L"uninstall")
    {
        return RunUninstall();
    }
    std::wstring root;
    DWORD error = ERROR_SUCCESS;
    if (!GetModuleDirectory(root, error))
    {
        return PrintWin32Failure(
            command == L"install" ? "install" : "plan-install", "module_path_failed", error, kExitInvalidPayload);
    }
    if (command == L"plan-install")
    {
        return RunPlanInstall(root);
    }
    if (command == L"install")
    {
        return RunInstall(root);
    }
    return PrintReport(
        "usage",
        false,
        "invalid_command",
        ERROR_INVALID_PARAMETER,
        L"Usage: LadoFlowWindowsSetup <self-test|plan-install|plan-uninstall|prepare-install|install|uninstall>",
        false,
        kExitUsage);
}
