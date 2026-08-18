#include "support.h"

#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>

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

  std::filesystem::remove_all(directory);
  return passed ? 0 : 1;
}
