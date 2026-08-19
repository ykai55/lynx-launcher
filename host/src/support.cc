#include "support.h"

#include <algorithm>
#include <cmath>
#include <cstdlib>
#include <fstream>
#include <iomanip>
#include <sstream>
#include <stdexcept>
#include <string_view>
#include <system_error>

#if defined(__linux__)
#include <unistd.h>
#endif

namespace launcher_host {

std::filesystem::path ExecutableDirectory(const char* argv0) {
#if defined(__linux__)
  std::vector<char> path(4096);
  const ssize_t length = readlink("/proc/self/exe", path.data(), path.size());
  if (length > 0 && static_cast<size_t>(length) < path.size()) {
    return std::filesystem::path(
               std::string(path.data(), static_cast<size_t>(length)))
        .parent_path();
  }
#endif
  if (argv0 && argv0[0] != '\0') {
    std::error_code error;
    auto absolute = std::filesystem::absolute(argv0, error);
    if (!error) {
      return absolute.parent_path();
    }
  }
  throw std::runtime_error("could not determine launcher executable directory");
}

std::filesystem::path RequireFile(
    const std::string& label, const std::string& explicit_path,
    const char* environment_name,
    const std::vector<std::filesystem::path>& candidates) {
  std::vector<std::filesystem::path> paths;
  if (!explicit_path.empty()) {
    paths.emplace_back(explicit_path);
  } else if (environment_name) {
    if (const char* value = std::getenv(environment_name); value && value[0]) {
      paths.emplace_back(value);
    }
  }
  if (paths.empty()) {
    paths = candidates;
  }

  for (const auto& path : paths) {
    std::error_code error;
    if (std::filesystem::is_regular_file(path, error) && !error) {
      auto canonical = std::filesystem::weakly_canonical(path, error);
      return error ? std::filesystem::absolute(path) : canonical;
    }
  }

  std::ostringstream message;
  message << "could not locate " << label;
  if (environment_name) {
    message << " (override with " << environment_name << ")";
  }
  message << "; checked:";
  for (const auto& path : paths) {
    message << "\n  " << path.string();
  }
  throw std::runtime_error(message.str());
}

std::vector<uint8_t> ReadFile(const std::filesystem::path& path) {
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  if (!input) {
    throw std::runtime_error("failed to open " + path.string());
  }
  const std::streamsize size = input.tellg();
  if (size <= 0) {
    throw std::runtime_error("resource is empty: " + path.string());
  }
  input.seekg(0, std::ios::beg);
  std::vector<uint8_t> data(static_cast<size_t>(size));
  if (!input.read(reinterpret_cast<char*>(data.data()), size)) {
    throw std::runtime_error("failed to read " + path.string());
  }
  return data;
}

std::string FileUri(const std::filesystem::path& path) {
  std::error_code error;
  auto absolute = std::filesystem::absolute(path, error);
  if (error) {
    absolute = path;
  }
  const std::string value = absolute.lexically_normal().generic_string();
  std::ostringstream uri;
  uri << "file://";
  uri << std::uppercase << std::hex << std::setfill('0');
  for (const unsigned char character : value) {
    const bool unreserved = (character >= 'a' && character <= 'z') ||
                            (character >= 'A' && character <= 'Z') ||
                            (character >= '0' && character <= '9') ||
                            character == '-' || character == '.' ||
                            character == '_' || character == '~' ||
                            character == '/';
    if (unreserved) {
      uri << static_cast<char>(character);
    } else {
      uri << '%' << std::setw(2) << static_cast<unsigned int>(character);
    }
  }
  return uri.str();
}

std::string Utf8FromCodepoint(uint32_t codepoint) {
  if (codepoint > 0x10ffff || (codepoint >= 0xd800 && codepoint <= 0xdfff)) {
    return {};
  }
  std::string result;
  if (codepoint <= 0x7f) {
    result.push_back(static_cast<char>(codepoint));
  } else if (codepoint <= 0x7ff) {
    result.push_back(static_cast<char>(0xc0 | (codepoint >> 6)));
    result.push_back(static_cast<char>(0x80 | (codepoint & 0x3f)));
  } else if (codepoint <= 0xffff) {
    result.push_back(static_cast<char>(0xe0 | (codepoint >> 12)));
    result.push_back(static_cast<char>(0x80 | ((codepoint >> 6) & 0x3f)));
    result.push_back(static_cast<char>(0x80 | (codepoint & 0x3f)));
  } else {
    result.push_back(static_cast<char>(0xf0 | (codepoint >> 18)));
    result.push_back(static_cast<char>(0x80 | ((codepoint >> 12) & 0x3f)));
    result.push_back(static_cast<char>(0x80 | ((codepoint >> 6) & 0x3f)));
    result.push_back(static_cast<char>(0x80 | (codepoint & 0x3f)));
  }
  return result;
}

double ScrollDeltaLogicalPixels(double offset) { return -offset * 100.0; }

std::optional<float> XSettingsWindowScale(std::span<const uint8_t> data) {
  if (data.size() < 12 || data[0] > 1) {
    return std::nullopt;
  }
  const bool little_endian = data[0] == 0;
  const auto read_u16 = [&](size_t offset) -> std::optional<uint16_t> {
    if (offset > data.size() || data.size() - offset < 2) {
      return std::nullopt;
    }
    if (little_endian) {
      return static_cast<uint16_t>(data[offset]) |
             static_cast<uint16_t>(data[offset + 1]) << 8;
    }
    return static_cast<uint16_t>(data[offset]) << 8 |
           static_cast<uint16_t>(data[offset + 1]);
  };
  const auto read_u32 = [&](size_t offset) -> std::optional<uint32_t> {
    if (offset > data.size() || data.size() - offset < 4) {
      return std::nullopt;
    }
    uint32_t value = 0;
    for (size_t index = 0; index < 4; ++index) {
      const size_t shift = little_endian ? index * 8 : (3 - index) * 8;
      value |= static_cast<uint32_t>(data[offset + index]) << shift;
    }
    return value;
  };
  const auto align_four = [&](size_t offset) -> std::optional<size_t> {
    const size_t padding = (4 - offset % 4) % 4;
    if (offset > data.size() || data.size() - offset < padding) {
      return std::nullopt;
    }
    return offset + padding;
  };

  const auto setting_count = read_u32(8);
  if (!setting_count) {
    return std::nullopt;
  }
  size_t offset = 12;
  for (uint32_t index = 0; index < *setting_count; ++index) {
    const auto name_length = read_u16(offset + 2);
    if (!name_length || offset > data.size() || data.size() - offset < 4) {
      return std::nullopt;
    }
    const uint8_t type = data[offset];
    offset += 4;
    if (data.size() - offset < *name_length) {
      return std::nullopt;
    }
    const std::string_view name(
        reinterpret_cast<const char*>(data.data() + offset), *name_length);
    const auto value_offset = align_four(offset + *name_length);
    if (!value_offset || !read_u32(*value_offset)) {
      return std::nullopt;
    }
    offset = *value_offset + 4;

    if (type == 0) {
      const auto value = read_u32(offset);
      if (!value) {
        return std::nullopt;
      }
      offset += 4;
      if (name == "Gdk/WindowScalingFactor") {
        if (*value >= 1 && *value <= 8) {
          return static_cast<float>(*value);
        }
        return std::nullopt;
      }
    } else if (type == 1) {
      const auto length = read_u32(offset);
      if (!length || data.size() - offset < 4 ||
          data.size() - (offset + 4) < *length) {
        return std::nullopt;
      }
      const auto next = align_four(offset + 4 + *length);
      if (!next) {
        return std::nullopt;
      }
      offset = *next;
    } else if (type == 2) {
      if (offset > data.size() || data.size() - offset < 8) {
        return std::nullopt;
      }
      offset += 8;
    } else {
      return std::nullopt;
    }
  }
  return std::nullopt;
}

std::optional<WindowMetrics> CalculateWindowMetrics(
    int window_width, int window_height, int framebuffer_width,
    int framebuffer_height, float system_scale) {
  if (window_width <= 0 || window_height <= 0 || framebuffer_width <= 0 ||
      framebuffer_height <= 0 || !std::isfinite(system_scale) ||
      system_scale <= 0) {
    return std::nullopt;
  }
  const float framebuffer_scale_x =
      static_cast<float>(framebuffer_width) / window_width;
  const float framebuffer_scale_y =
      static_cast<float>(framebuffer_height) / window_height;
  const float pixel_ratio =
      std::max({system_scale, framebuffer_scale_x, framebuffer_scale_y});
  return WindowMetrics{
      .logical_width = framebuffer_width / pixel_ratio,
      .logical_height = framebuffer_height / pixel_ratio,
      .pixel_ratio = pixel_ratio,
      .framebuffer_scale_x = framebuffer_scale_x,
      .framebuffer_scale_y = framebuffer_scale_y,
  };
}

}  // namespace launcher_host
