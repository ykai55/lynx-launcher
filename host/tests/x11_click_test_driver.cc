#include <X11/Xatom.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/keysym.h>

#include <algorithm>
#include <charconv>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <iostream>
#include <limits>
#include <optional>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

#include "support.h"

namespace {

constexpr int kLauncherLogicalWidth = 1120;
constexpr int kLauncherLogicalHeight = 760;

enum class Action { kClick, kType, kExpectPixel, kExpectRegionsDiffer };

struct Options {
  unsigned long pid = 0;
  Action action = Action::kClick;
  std::optional<int> x;
  std::optional<int> y;
  std::optional<int> other_x;
  std::optional<int> other_y;
  std::optional<int> width;
  std::optional<int> height;
  std::optional<int> red;
  std::optional<int> green;
  std::optional<int> blue;
  std::optional<int> tolerance;
  std::optional<int> minimum_matches;
  std::optional<int> minimum_differences;
  std::optional<int> timeout_ms;
  std::optional<std::string> text;
};

const char* Usage() {
  return "usage:\n"
         "  x11_click_test_driver --pid PID click --x X --y Y\n"
         "  x11_click_test_driver --pid PID type --text ASCII\n"
         "  x11_click_test_driver --pid PID expect-pixel --x X --y Y "
         "--width W --height H --red R --green G --blue B --tolerance N "
         "--minimum-matches N --timeout-ms N\n"
         "  x11_click_test_driver --pid PID expect-regions-differ --x X "
         "--y Y --other-x X --other-y Y --width W --height H --red R "
         "--green G --blue B --tolerance N --minimum-differences N "
         "--timeout-ms N";
}

uint64_t ParseUnsigned(const std::string& value, const std::string& name,
                       uint64_t maximum) {
  uint64_t result = 0;
  const auto [end, error] =
      std::from_chars(value.data(), value.data() + value.size(), result);
  if (value.empty() || error != std::errc{} ||
      end != value.data() + value.size() || result > maximum) {
    throw std::runtime_error(name + " must be an unsigned integer from 0 to " +
                             std::to_string(maximum));
  }
  return result;
}

void SetOnce(std::optional<int>& destination, const std::string& value,
             const std::string& name, int maximum) {
  if (destination) {
    throw std::runtime_error("duplicate option: " + name);
  }
  destination = static_cast<int>(ParseUnsigned(value, name, maximum));
}

Options ParseOptions(int argc, char** argv) {
  if (argc < 4 || std::string(argv[1]) != "--pid") {
    throw std::runtime_error(Usage());
  }
  Options options;
  options.pid = static_cast<unsigned long>(
      ParseUnsigned(argv[2], "--pid", std::numeric_limits<uint32_t>::max()));
  if (options.pid == 0) {
    throw std::runtime_error("--pid must be greater than zero");
  }

  const std::string action = argv[3];
  if (action == "click") {
    options.action = Action::kClick;
  } else if (action == "type") {
    options.action = Action::kType;
  } else if (action == "expect-pixel") {
    options.action = Action::kExpectPixel;
  } else if (action == "expect-regions-differ") {
    options.action = Action::kExpectRegionsDiffer;
  } else {
    throw std::runtime_error("unknown action: " + action + "\n" + Usage());
  }

  for (int index = 4; index < argc; index += 2) {
    if (index + 1 >= argc) {
      throw std::runtime_error("missing value after " +
                               std::string(argv[index]));
    }
    const std::string name = argv[index];
    const std::string value = argv[index + 1];
    if (name == "--x") {
      SetOnce(options.x, value, name, 100000);
    } else if (name == "--y") {
      SetOnce(options.y, value, name, 100000);
    } else if (name == "--other-x") {
      SetOnce(options.other_x, value, name, 100000);
    } else if (name == "--other-y") {
      SetOnce(options.other_y, value, name, 100000);
    } else if (name == "--width") {
      SetOnce(options.width, value, name, 10000);
    } else if (name == "--height") {
      SetOnce(options.height, value, name, 10000);
    } else if (name == "--red") {
      SetOnce(options.red, value, name, 255);
    } else if (name == "--green") {
      SetOnce(options.green, value, name, 255);
    } else if (name == "--blue") {
      SetOnce(options.blue, value, name, 255);
    } else if (name == "--tolerance") {
      SetOnce(options.tolerance, value, name, 255);
    } else if (name == "--minimum-matches") {
      SetOnce(options.minimum_matches, value, name, 100000000);
    } else if (name == "--minimum-differences") {
      SetOnce(options.minimum_differences, value, name, 100000000);
    } else if (name == "--timeout-ms") {
      SetOnce(options.timeout_ms, value, name, 60000);
    } else if (name == "--text") {
      if (options.text) {
        throw std::runtime_error("duplicate option: --text");
      }
      options.text = value;
    } else {
      throw std::runtime_error("unknown option: " + name);
    }
  }

  if (options.action == Action::kClick) {
    if (!options.x || !options.y || argc != 8) {
      throw std::runtime_error(Usage());
    }
  } else if (options.action == Action::kType) {
    if (!options.text || argc != 6 || options.text->empty() ||
        options.text->size() > 128 ||
        !std::all_of(options.text->begin(), options.text->end(),
                     [](char value) {
                       return (value >= 'a' && value <= 'z') ||
                              (value >= 'A' && value <= 'Z') ||
                              (value >= '0' && value <= '9') || value == ' ';
                     })) {
      throw std::runtime_error(
          "type requires 1-128 ASCII letters, digits, or spaces");
    }
  } else if (options.action == Action::kExpectPixel) {
    if (!options.x || !options.y || !options.width || !options.height ||
        !options.red || !options.green || !options.blue || !options.tolerance ||
        !options.minimum_matches || !options.timeout_ms || argc != 24 ||
        *options.width == 0 || *options.height == 0 ||
        *options.minimum_matches == 0 || *options.timeout_ms == 0 ||
        static_cast<uint64_t>(*options.minimum_matches) >
            static_cast<uint64_t>(*options.width) * *options.height) {
      throw std::runtime_error(Usage());
    }
  } else if (!options.x || !options.y || !options.other_x ||
             !options.other_y || !options.width || !options.height ||
             !options.red || !options.green || !options.blue ||
             !options.tolerance || !options.minimum_differences ||
             !options.timeout_ms || argc != 28 || *options.width == 0 ||
             *options.height == 0 || *options.minimum_differences == 0 ||
             *options.timeout_ms == 0 ||
             static_cast<uint64_t>(*options.minimum_differences) >
                 static_cast<uint64_t>(*options.width) * *options.height) {
    throw std::runtime_error(Usage());
  }
  return options;
}

unsigned long WindowPid(Display* display, Window window, Atom pid_atom) {
  Atom actual_type = None;
  int actual_format = 0;
  unsigned long count = 0;
  unsigned long remaining = 0;
  unsigned char* data = nullptr;
  const int status = XGetWindowProperty(
      display, window, pid_atom, 0, 1, False, XA_CARDINAL, &actual_type,
      &actual_format, &count, &remaining, &data);
  unsigned long pid = 0;
  if (status == Success && actual_type == XA_CARDINAL && actual_format == 32 &&
      count == 1 && data) {
    pid = *reinterpret_cast<unsigned long*>(data);
  }
  if (data) {
    XFree(data);
  }
  return pid;
}

void FindWindows(Display* display, Window parent, Atom pid_atom,
                 unsigned long pid, std::vector<Window>& matches) {
  if (WindowPid(display, parent, pid_atom) == pid) {
    XWindowAttributes attributes{};
    if (XGetWindowAttributes(display, parent, &attributes) &&
        attributes.c_class == InputOutput &&
        attributes.map_state == IsViewable) {
      matches.push_back(parent);
    }
  }
  Window root = None;
  Window owner = None;
  Window* children = nullptr;
  unsigned int child_count = 0;
  if (!XQueryTree(display, parent, &root, &owner, &children, &child_count)) {
    return;
  }
  for (unsigned int index = 0; index < child_count; ++index) {
    FindWindows(display, children[index], pid_atom, pid, matches);
  }
  if (children) {
    XFree(children);
  }
}

std::optional<float> XSettingsScale(Display* display) {
  const std::string selection_name =
      "_XSETTINGS_S" + std::to_string(DefaultScreen(display));
  const Atom selection = XInternAtom(display, selection_name.c_str(), True);
  const Atom property = XInternAtom(display, "_XSETTINGS_SETTINGS", True);
  if (selection == None || property == None) {
    return std::nullopt;
  }
  const Window owner = XGetSelectionOwner(display, selection);
  if (owner == None) {
    return std::nullopt;
  }

  Atom actual_type = None;
  int actual_format = 0;
  unsigned long item_count = 0;
  unsigned long remaining = 0;
  unsigned char* data = nullptr;
  const int status = XGetWindowProperty(
      display, owner, property, 0, 65536, False, property, &actual_type,
      &actual_format, &item_count, &remaining, &data);
  std::optional<float> scale;
  if (status == Success && actual_type == property && actual_format == 8 &&
      remaining == 0 && data) {
    scale = launcher_host::XSettingsWindowScale(
        std::span<const uint8_t>(data, item_count));
  }
  if (data) {
    XFree(data);
  }
  return scale;
}

void RequireSent(int status, const char* event_name) {
  if (status == 0) {
    throw std::runtime_error(std::string("XSendEvent failed for ") +
                             event_name);
  }
}

void SendClick(Display* display, Window root, Window window, int root_x,
               int root_y, int x, int y) {
  XEvent event{};
  event.xcrossing.type = EnterNotify;
  event.xcrossing.display = display;
  event.xcrossing.window = window;
  event.xcrossing.root = root;
  event.xcrossing.time = CurrentTime;
  event.xcrossing.x = x;
  event.xcrossing.y = y;
  event.xcrossing.x_root = root_x;
  event.xcrossing.y_root = root_y;
  event.xcrossing.mode = NotifyNormal;
  event.xcrossing.detail = NotifyNonlinear;
  event.xcrossing.same_screen = True;
  event.xcrossing.focus = True;
  RequireSent(XSendEvent(display, window, False, EnterWindowMask, &event),
              "EnterNotify");

  event = {};
  event.xmotion.type = MotionNotify;
  event.xmotion.display = display;
  event.xmotion.window = window;
  event.xmotion.root = root;
  event.xmotion.time = CurrentTime;
  event.xmotion.x = x;
  event.xmotion.y = y;
  event.xmotion.x_root = root_x;
  event.xmotion.y_root = root_y;
  event.xmotion.same_screen = True;
  RequireSent(XSendEvent(display, window, False, PointerMotionMask, &event),
              "MotionNotify");

  event = {};
  event.xbutton.type = ButtonPress;
  event.xbutton.display = display;
  event.xbutton.window = window;
  event.xbutton.root = root;
  event.xbutton.time = CurrentTime;
  event.xbutton.x = x;
  event.xbutton.y = y;
  event.xbutton.x_root = root_x;
  event.xbutton.y_root = root_y;
  event.xbutton.button = Button1;
  event.xbutton.same_screen = True;
  RequireSent(XSendEvent(display, window, False, ButtonPressMask, &event),
              "ButtonPress");

  event.xbutton.type = ButtonRelease;
  event.xbutton.state = Button1Mask;
  RequireSent(XSendEvent(display, window, False, ButtonReleaseMask, &event),
              "ButtonRelease");
  XSync(display, False);
}

void SendText(Display* display, Window root, Window window,
              const std::string& text) {
  for (const char character : text) {
    const bool shifted = character >= 'A' && character <= 'Z';
    const char key =
        shifted ? static_cast<char>(character - 'A' + 'a') : character;
    const KeySym symbol = key == ' ' ? XK_space : static_cast<KeySym>(key);
    const KeyCode keycode = XKeysymToKeycode(display, symbol);
    if (keycode == 0) {
      throw std::runtime_error("the active X11 keymap cannot type the query");
    }

    XEvent event{};
    event.xkey.type = KeyPress;
    event.xkey.display = display;
    event.xkey.window = window;
    event.xkey.root = root;
    event.xkey.time = CurrentTime;
    event.xkey.state = shifted ? ShiftMask : 0;
    event.xkey.keycode = keycode;
    event.xkey.same_screen = True;
    RequireSent(XSendEvent(display, window, False, KeyPressMask, &event),
                "KeyPress");
    event.xkey.type = KeyRelease;
    RequireSent(XSendEvent(display, window, False, KeyReleaseMask, &event),
                "KeyRelease");
    XSync(display, False);
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
  }
}

int PixelChannel(unsigned long pixel, unsigned long mask) {
  if (mask == 0) {
    throw std::runtime_error("X11 image has an unsupported color mask");
  }
  unsigned int shift = 0;
  while ((mask & 1UL) == 0) {
    mask >>= 1;
    ++shift;
  }
  const unsigned long value = (pixel >> shift) & mask;
  return static_cast<int>((value * 255UL + mask / 2UL) / mask);
}

int MatchingPixels(Display* display, Window window, const Options& options) {
  XImage* image =
      XGetImage(display, window, *options.x, *options.y, *options.width,
                *options.height, AllPlanes, ZPixmap);
  if (!image) {
    throw std::runtime_error("XGetImage could not read the GLFW window");
  }
  int matches = 0;
  for (int y = 0; y < image->height; ++y) {
    for (int x = 0; x < image->width; ++x) {
      const unsigned long pixel = XGetPixel(image, x, y);
      const int red = PixelChannel(pixel, image->red_mask);
      const int green = PixelChannel(pixel, image->green_mask);
      const int blue = PixelChannel(pixel, image->blue_mask);
      if (std::abs(red - *options.red) <= *options.tolerance &&
          std::abs(green - *options.green) <= *options.tolerance &&
          std::abs(blue - *options.blue) <= *options.tolerance) {
        ++matches;
      }
    }
  }
  XDestroyImage(image);
  return matches;
}

void ExpectPixel(Display* display, Window window, const Options& options) {
  const auto deadline = std::chrono::steady_clock::now() +
                        std::chrono::milliseconds(*options.timeout_ms);
  int best_match_count = 0;
  do {
    best_match_count =
        std::max(best_match_count, MatchingPixels(display, window, options));
    if (best_match_count >= *options.minimum_matches) {
      std::cout << "matched " << best_match_count
                << " rendered pixels near rgb(" << *options.red << ','
                << *options.green << ',' << *options.blue << ")\n";
      return;
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(50));
  } while (std::chrono::steady_clock::now() < deadline);
  throw std::runtime_error(
      "rendered pixel assertion found " + std::to_string(best_match_count) +
      ", expected at least " + std::to_string(*options.minimum_matches));
}

int DifferingPixels(Display* display, Window window, const Options& options) {
  XImage* first = XGetImage(display, window, *options.x, *options.y,
                            *options.width, *options.height, AllPlanes, ZPixmap);
  XImage* second =
      XGetImage(display, window, *options.other_x, *options.other_y,
                *options.width, *options.height, AllPlanes, ZPixmap);
  if (!first || !second) {
    if (first) {
      XDestroyImage(first);
    }
    if (second) {
      XDestroyImage(second);
    }
    throw std::runtime_error("XGetImage could not read both comparison regions");
  }

  int differences = 0;
  for (int y = 0; y < first->height; ++y) {
    for (int x = 0; x < first->width; ++x) {
      const auto is_target = [&](XImage* image) {
        const unsigned long pixel = XGetPixel(image, x, y);
        return std::abs(PixelChannel(pixel, image->red_mask) - *options.red) <=
                   *options.tolerance &&
               std::abs(PixelChannel(pixel, image->green_mask) -
                        *options.green) <= *options.tolerance &&
               std::abs(PixelChannel(pixel, image->blue_mask) - *options.blue) <=
                   *options.tolerance;
      };
      if (is_target(first) != is_target(second)) {
        ++differences;
      }
    }
  }
  XDestroyImage(first);
  XDestroyImage(second);
  return differences;
}

void ExpectRegionsDiffer(Display* display, Window window,
                         const Options& options) {
  const auto deadline = std::chrono::steady_clock::now() +
                        std::chrono::milliseconds(*options.timeout_ms);
  int best_difference_count = 0;
  do {
    best_difference_count = std::max(
        best_difference_count, DifferingPixels(display, window, options));
    if (best_difference_count >= *options.minimum_differences) {
      std::cout << "found " << best_difference_count
                << " differing rendered text pixels\n";
      return;
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(50));
  } while (std::chrono::steady_clock::now() < deadline);
  throw std::runtime_error(
      "rendered text regions differed at " +
      std::to_string(best_difference_count) + " pixels, expected at least " +
      std::to_string(*options.minimum_differences));
}

}  // namespace

int main(int argc, char** argv) {
  try {
    const Options options = ParseOptions(argc, argv);
    Display* display = XOpenDisplay(nullptr);
    if (!display) {
      throw std::runtime_error("could not open DISPLAY");
    }
    struct DisplayGuard {
      Display* display;
      ~DisplayGuard() { XCloseDisplay(display); }
    } display_guard{display};

    const Window root = DefaultRootWindow(display);
    const Atom pid_atom = XInternAtom(display, "_NET_WM_PID", False);
    std::vector<Window> matches;
    FindWindows(display, root, pid_atom, options.pid, matches);
    if (matches.size() != 1) {
      throw std::runtime_error(
          "expected exactly one visible X11 window for PID " +
          std::to_string(options.pid) + ", found " +
          std::to_string(matches.size()));
    }
    const Window window = matches.front();
    XWindowAttributes attributes{};
    if (!XGetWindowAttributes(display, window, &attributes)) {
      throw std::runtime_error("could not read X11 window attributes");
    }

    if (options.action == Action::kType) {
      SendText(display, root, window, *options.text);
      return 0;
    }

    Options physical = options;
    const double inferred_scale = std::min(
        static_cast<double>(attributes.width) / kLauncherLogicalWidth,
        static_cast<double>(attributes.height) / kLauncherLogicalHeight);
    const double scale = XSettingsScale(display).value_or(inferred_scale);
    const double scale_x = scale;
    const double scale_y = scale;
    physical.x = static_cast<int>(std::lround(*options.x * scale_x));
    physical.y = static_cast<int>(std::lround(*options.y * scale_y));
    if (options.other_x) {
      physical.other_x =
          static_cast<int>(std::lround(*options.other_x * scale_x));
      physical.other_y =
          static_cast<int>(std::lround(*options.other_y * scale_y));
    }
    if (*physical.x >= attributes.width || *physical.y >= attributes.height) {
      throw std::runtime_error(
          "coordinates are outside the specified PID window");
    }
    if (options.action == Action::kExpectPixel ||
        options.action == Action::kExpectRegionsDiffer) {
      physical.width =
          std::max(1, static_cast<int>(std::lround(*options.width * scale_x)));
      physical.height =
          std::max(1, static_cast<int>(std::lround(*options.height * scale_y)));
      if (options.minimum_matches) {
        physical.minimum_matches = std::max(
            1, static_cast<int>(
                   std::lround(*options.minimum_matches * scale_x * scale_y)));
      }
      if (options.minimum_differences) {
        physical.minimum_differences = std::max(
            1, static_cast<int>(std::lround(*options.minimum_differences *
                                            scale_x * scale_y)));
      }
      if (*physical.width > attributes.width - *physical.x ||
          *physical.height > attributes.height - *physical.y) {
        throw std::runtime_error(
            "pixel region is outside the specified PID window");
      }
      if (options.action == Action::kExpectPixel) {
        ExpectPixel(display, window, physical);
        return 0;
      }
      if (*physical.other_x >= attributes.width ||
          *physical.other_y >= attributes.height ||
          *physical.width > attributes.width - *physical.other_x ||
          *physical.height > attributes.height - *physical.other_y) {
        throw std::runtime_error(
            "comparison region is outside the specified PID window");
      }
      ExpectRegionsDiffer(display, window, physical);
      return 0;
    }

    Window translated_child = None;
    int window_root_x = 0;
    int window_root_y = 0;
    if (!XTranslateCoordinates(display, window, root, 0, 0, &window_root_x,
                               &window_root_y, &translated_child)) {
      throw std::runtime_error("could not translate window coordinates");
    }
    SendClick(display, root, window, window_root_x + *physical.x,
              window_root_y + *physical.y, *physical.x, *physical.y);
    return 0;
  } catch (const std::exception& error) {
    std::cerr << "x11_click_test_driver: " << error.what() << '\n';
    return 1;
  }
}
