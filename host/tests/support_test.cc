#include "support.h"

#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

namespace {

bool Expect(bool condition, const char* message) {
  if (!condition) {
    std::cerr << "FAIL: " << message << '\n';
  }
  return condition;
}

}  // namespace

int main() {
  bool passed = true;
  const auto directory = std::filesystem::temp_directory_path() /
                         "lynx launcher host support test";
  std::filesystem::create_directories(directory);
  const auto file = directory / "icon #1.png";
  {
    std::ofstream output(file, std::ios::binary);
    output << "png";
  }

  const auto resolved =
      launcher_host::RequireFile("test file", file.string(), nullptr, {});
  passed &= Expect(resolved == std::filesystem::canonical(file),
                   "RequireFile returns the canonical explicit path");
  passed &= Expect(launcher_host::ReadFile(file).size() == 3,
                   "ReadFile preserves binary bytes");
  const std::string uri = launcher_host::FileUri(file);
  passed &= Expect(uri.starts_with("file:///"), "FileUri is an absolute URI");
  passed &= Expect(uri.find("lynx%20launcher") != std::string::npos,
                   "FileUri escapes spaces");
  passed &= Expect(uri.find("%231.png") != std::string::npos,
                   "FileUri escapes reserved characters");
  passed &= Expect(launcher_host::Utf8FromCodepoint('A') == "A",
                   "ASCII codepoints encode as UTF-8");
  passed &= Expect(launcher_host::Utf8FromCodepoint(0x4e2d) == "\xe4\xb8\xad",
                   "BMP Unicode codepoints encode as UTF-8");
  passed &=
      Expect(launcher_host::Utf8FromCodepoint(0x1f680) == "\xf0\x9f\x9a\x80",
             "supplementary Unicode codepoints encode as UTF-8");
  passed &= Expect(launcher_host::Utf8FromCodepoint(0xd800).empty(),
                   "UTF-16 surrogates are rejected");
  passed &= Expect(launcher_host::Utf8FromCodepoint(0x110000).empty(),
                    "out-of-range Unicode codepoints are rejected");

  std::vector<uint8_t> xsettings{0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0};
  const std::string scale_name = "Gdk/WindowScalingFactor";
  xsettings.push_back(0);
  xsettings.push_back(0);
  xsettings.push_back(static_cast<uint8_t>(scale_name.size()));
  xsettings.push_back(0);
  xsettings.insert(xsettings.end(), scale_name.begin(), scale_name.end());
  while (xsettings.size() % 4 != 0) {
    xsettings.push_back(0);
  }
  xsettings.insert(xsettings.end(), {1, 0, 0, 0, 2, 0, 0, 0});
  const auto window_scale = launcher_host::XSettingsWindowScale(xsettings);
  passed &= Expect(window_scale && *window_scale == 2.0f,
                    "XSettingsWindowScale reads the GNOME window scale");
  passed &= Expect(
      !launcher_host::XSettingsWindowScale(
           std::span<const uint8_t>(xsettings).first(11))
           .has_value(),
      "XSettingsWindowScale rejects a truncated property");

  const auto scaled_metrics = launcher_host::CalculateWindowMetrics(
      2240, 1520, 2240, 1520, 2.0f);
  passed &= Expect(
      scaled_metrics && scaled_metrics->logical_width == 1120.0f &&
          scaled_metrics->logical_height == 760.0f &&
          scaled_metrics->pixel_ratio == 2.0f &&
          scaled_metrics->framebuffer_scale_x == 1.0f &&
          scaled_metrics->framebuffer_scale_y == 1.0f,
      "system scaling preserves logical size and physical pointer coordinates");
  const auto framebuffer_metrics = launcher_host::CalculateWindowMetrics(
      1120, 760, 2240, 1520, 1.0f);
  passed &= Expect(
      framebuffer_metrics && framebuffer_metrics->logical_width == 1120.0f &&
          framebuffer_metrics->logical_height == 760.0f &&
          framebuffer_metrics->pixel_ratio == 2.0f &&
          framebuffer_metrics->framebuffer_scale_x == 2.0f &&
          framebuffer_metrics->framebuffer_scale_y == 2.0f,
      "framebuffer scaling remains supported independently");

  std::filesystem::remove_all(directory);
  return passed ? 0 : 1;
}
