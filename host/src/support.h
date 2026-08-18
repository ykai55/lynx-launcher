#ifndef LYNX_LAUNCHER_HOST_SUPPORT_H_
#define LYNX_LAUNCHER_HOST_SUPPORT_H_

#include <cstdint>
#include <filesystem>
#include <optional>
#include <span>
#include <string>
#include <vector>

namespace launcher_host {

struct WindowMetrics {
  float logical_width;
  float logical_height;
  float pixel_ratio;
  float framebuffer_scale_x;
  float framebuffer_scale_y;
};

std::filesystem::path ExecutableDirectory(const char* argv0);
std::filesystem::path RequireFile(
    const std::string& label, const std::string& explicit_path,
    const char* environment_name,
    const std::vector<std::filesystem::path>& candidates);
std::vector<uint8_t> ReadFile(const std::filesystem::path& path);
std::string FileUri(const std::filesystem::path& path);
std::string Utf8FromCodepoint(uint32_t codepoint);
std::optional<float> XSettingsWindowScale(std::span<const uint8_t> data);
std::optional<WindowMetrics> CalculateWindowMetrics(
    int window_width, int window_height, int framebuffer_width,
    int framebuffer_height, float system_scale);

}  // namespace launcher_host

#endif  // LYNX_LAUNCHER_HOST_SUPPORT_H_
