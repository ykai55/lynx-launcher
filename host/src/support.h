#ifndef LYNX_LAUNCHER_HOST_SUPPORT_H_
#define LYNX_LAUNCHER_HOST_SUPPORT_H_

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

namespace launcher_host {

std::filesystem::path ExecutableDirectory(const char* argv0);
std::filesystem::path RequireFile(
    const std::string& label, const std::string& explicit_path,
    const char* environment_name,
    const std::vector<std::filesystem::path>& candidates);
std::vector<uint8_t> ReadFile(const std::filesystem::path& path);
std::string FileUri(const std::filesystem::path& path);
std::string Utf8FromCodepoint(uint32_t codepoint);

}  // namespace launcher_host

#endif  // LYNX_LAUNCHER_HOST_SUPPORT_H_
