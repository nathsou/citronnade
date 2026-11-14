// Installs a CIA with Azahar's own installer (Service::AM::InstallCIA, the
// code its frontends call), for checking the CIAs cargo-citronnade makes.
// run.c calls this once the core runs: the installer needs the system up.

#include <cstdint>
#include <cstdio>
#include <dlfcn.h>
#include <functional>
#include <string>

namespace {
enum class InstallStatus : uint32_t { Success, ErrorFailedToOpenFile, ErrorFileNotFound, ErrorAborted, ErrorInvalid, ErrorEncrypted };
enum class MediaType : uint32_t { NAND, SDMC, GameCard };
using InstallCia = InstallStatus (*)(const std::string&, std::function<void(unsigned long, unsigned long)>&&);
using ContentPath = std::string (*)(MediaType, uint64_t, uint64_t, bool);
const char* const STATUS[] = {"installed", "could not open the file", "file not found", "aborted", "invalid", "encrypted"};
}

// Installs `path`; prints the result and, given the title ID, the path of
// the installed content 0 (an NCCH the core can run). Returns 0 on success.
extern "C" int emu_install_cia(void* core, const char* path, uint64_t title_id) {
    auto install = reinterpret_cast<InstallCia>(
        dlsym(core, "_ZN7Service2AM10InstallCIAERKNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEEEOSt8functionIFvmmEE"));
    auto content = reinterpret_cast<ContentPath>(dlsym(core, "_ZN7Service2AM19GetTitleContentPathB5cxx11ENS_2FS9MediaTypeEmmb"));
    if (!install || !content) {
        std::printf("[harness] the core does not export InstallCIA\n");
        return 1;
    }
    InstallStatus status = install(std::string(path), {});
    auto s = static_cast<uint32_t>(status);
    std::printf("[harness] install %s: %s\n", path, s < 6 ? STATUS[s] : "unknown status");
    if (status == InstallStatus::Success && title_id != 0) {
        std::printf("[harness] content 0 of %016llX: %s\n", (unsigned long long)title_id,
                    content(MediaType::SDMC, title_id, 0, false).c_str());
    }
    std::fflush(stdout);
    return status == InstallStatus::Success ? 0 : 1;
}
