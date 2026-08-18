#include "support.h"

#include <cstdlib>
#include <fstream>
#include <iomanip>
#include <sstream>
#include <stdexcept>
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

}  // namespace launcher_host
