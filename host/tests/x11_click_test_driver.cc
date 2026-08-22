#include <X11/Xatom.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/keysym.h>

#include <sys/wait.h>
#include <unistd.h>

#include <fcntl.h>
#include <poll.h>
#include <signal.h>

#include <algorithm>
#include <charconv>
#include <chrono>
#include <cctype>
#include <cerrno>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <iostream>
#include <limits>
#include <optional>
#include <stdexcept>
#include <string>
#include <thread>
#include <utility>
#include <vector>

#include "support.h"

namespace {

constexpr int kLauncherLogicalWidth = 1120;
constexpr int kLauncherLogicalHeight = 760;
constexpr double kMaximumScale = 8.0;
constexpr double kMaximumCompositorCoordinate = 1'000'000.0;
constexpr double kMaximumCompositorDimension = 32'768.0;
using Deadline = std::chrono::steady_clock::time_point;

enum class Action {
  kClick,
  kDefocus,
  kExpectPopup,
  kType,
  kExpectPixel,
  kExpectNoPixel,
  kExpectRegionsDiffer,
  kScroll,
  kRepeatKey,
  kHoldInput,
};

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
  std::optional<int> maximum_matches;
  std::optional<int> minimum_differences;
  std::optional<int> timeout_ms;
  std::optional<std::string> text;
  std::optional<std::string> direction;
  std::optional<std::string> key;
  std::optional<double> scale;
};

const char* Usage() {
  return "usage:\n"
         "  x11_click_test_driver --self-test-server-clock\n"
         "  x11_click_test_driver --self-test-visible-pixels\n"
         "  x11_click_test_driver --pid PID click --x X --y Y "
         "[--scale SCALE]\n"
         "  x11_click_test_driver --pid PID defocus\n"
         "  x11_click_test_driver --pid PID expect-popup\n"
         "  x11_click_test_driver --pid PID type --text ASCII\n"
         "  x11_click_test_driver --pid PID scroll --x X --y Y "
         "--direction up|down [--scale SCALE]\n"
         "  x11_click_test_driver --pid PID repeat-key --key left-shift\n"
         "  x11_click_test_driver --pid PID hold-input --key left-shift "
         "--x X --y Y [--scale SCALE]\n"
         "  x11_click_test_driver --pid PID expect-pixel --x X --y Y "
         "--width W --height H --red R --green G --blue B --tolerance N "
         "--minimum-matches N --timeout-ms N [--scale SCALE]\n"
         "  x11_click_test_driver --pid PID expect-no-pixel --x X --y Y "
         "--width W --height H --red R --green G --blue B --tolerance N "
         "--maximum-matches N --timeout-ms N [--scale SCALE]\n"
         "  x11_click_test_driver --pid PID expect-regions-differ --x X "
         "--y Y --other-x X --other-y Y --width W --height H --red R "
         "--green G --blue B --tolerance N --minimum-differences N "
         "--timeout-ms N [--scale SCALE]";
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

double ParseScale(const std::string& value) {
  double result = 0;
  const auto [end, error] =
      std::from_chars(value.data(), value.data() + value.size(), result);
  if (value.empty() || error != std::errc{} ||
      end != value.data() + value.size() || !std::isfinite(result) ||
      result < 1.0 || result > 8.0) {
    throw std::runtime_error("--scale must be a finite number from 1 to 8");
  }
  return result;
}

bool ExecutableOnPath(const std::string& command) {
  const char* path = std::getenv("PATH");
  if (!path) {
    return false;
  }
  std::string paths(path);
  size_t begin = 0;
  for (;;) {
    const size_t end = paths.find(':', begin);
    const std::string directory =
        paths.substr(begin, end == std::string::npos ? end : end - begin);
    const std::string candidate =
        (directory.empty() ? "." : directory) + "/" + command;
    if (access(candidate.c_str(), X_OK) == 0) {
      return true;
    }
    if (end == std::string::npos) {
      return false;
    }
    begin = end + 1;
  }
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
  } else if (action == "defocus") {
    options.action = Action::kDefocus;
  } else if (action == "expect-popup") {
    options.action = Action::kExpectPopup;
  } else if (action == "type") {
    options.action = Action::kType;
  } else if (action == "expect-pixel") {
    options.action = Action::kExpectPixel;
  } else if (action == "expect-no-pixel") {
    options.action = Action::kExpectNoPixel;
  } else if (action == "expect-regions-differ") {
    options.action = Action::kExpectRegionsDiffer;
  } else if (action == "scroll") {
    options.action = Action::kScroll;
  } else if (action == "repeat-key") {
    options.action = Action::kRepeatKey;
  } else if (action == "hold-input") {
    options.action = Action::kHoldInput;
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
    } else if (name == "--maximum-matches") {
      SetOnce(options.maximum_matches, value, name, 100000000);
    } else if (name == "--minimum-differences") {
      SetOnce(options.minimum_differences, value, name, 100000000);
    } else if (name == "--timeout-ms") {
      SetOnce(options.timeout_ms, value, name, 60000);
    } else if (name == "--text") {
      if (options.text) {
        throw std::runtime_error("duplicate option: --text");
      }
      options.text = value;
    } else if (name == "--direction") {
      if (options.direction) {
        throw std::runtime_error("duplicate option: --direction");
      }
      options.direction = value;
    } else if (name == "--key") {
      if (options.key) {
        throw std::runtime_error("duplicate option: --key");
      }
      options.key = value;
    } else if (name == "--scale") {
      if (options.scale) {
        throw std::runtime_error("duplicate option: --scale");
      }
      options.scale = ParseScale(value);
    } else {
      throw std::runtime_error("unknown option: " + name);
    }
  }

  if (options.action == Action::kDefocus ||
      options.action == Action::kExpectPopup) {
    if (argc != 4) {
      throw std::runtime_error(Usage());
    }
  } else if (options.action == Action::kClick) {
    if (!options.x || !options.y || argc != 8 + (options.scale ? 2 : 0)) {
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
  } else if (options.action == Action::kScroll) {
    if (!options.x || !options.y || !options.direction ||
        (*options.direction != "up" && *options.direction != "down") ||
        argc != 10 + (options.scale ? 2 : 0)) {
      throw std::runtime_error(Usage());
    }
  } else if (options.action == Action::kRepeatKey) {
    if (!options.key || *options.key != "left-shift" || argc != 6) {
      throw std::runtime_error(Usage());
    }
  } else if (options.action == Action::kHoldInput) {
    if (!options.key || *options.key != "left-shift" || !options.x || !options.y ||
        argc != 10 + (options.scale ? 2 : 0)) {
      throw std::runtime_error(Usage());
    }
  } else if (options.action == Action::kExpectPixel ||
             options.action == Action::kExpectNoPixel) {
    if (!options.x || !options.y || !options.width || !options.height ||
        !options.red || !options.green || !options.blue || !options.tolerance ||
        !options.timeout_ms || argc != 24 + (options.scale ? 2 : 0) ||
        *options.width == 0 ||
        *options.height == 0 || *options.timeout_ms == 0 ||
        (options.action == Action::kExpectPixel &&
         (!options.minimum_matches || *options.minimum_matches == 0 ||
          static_cast<uint64_t>(*options.minimum_matches) >
              static_cast<uint64_t>(*options.width) * *options.height)) ||
        (options.action == Action::kExpectNoPixel &&
         (!options.maximum_matches ||
          static_cast<uint64_t>(*options.maximum_matches) >
              static_cast<uint64_t>(*options.width) * *options.height))) {
      throw std::runtime_error(Usage());
    }
  } else if (!options.x || !options.y || !options.other_x ||
             !options.other_y || !options.width || !options.height ||
             !options.red || !options.green || !options.blue ||
             !options.tolerance || !options.minimum_differences ||
              !options.timeout_ms ||
              argc != 28 + (options.scale ? 2 : 0) || *options.width == 0 ||
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

std::vector<Atom> AtomProperty(Display* display, Window window,
                               const char* name) {
  const Atom property = XInternAtom(display, name, True);
  if (property == None) {
    return {};
  }
  Atom actual_type = None;
  int actual_format = 0;
  unsigned long count = 0;
  unsigned long remaining = 0;
  unsigned char* data = nullptr;
  const int status = XGetWindowProperty(
      display, window, property, 0, 64, False, XA_ATOM, &actual_type,
      &actual_format, &count, &remaining, &data);
  std::vector<Atom> atoms;
  if (status == Success && actual_type == XA_ATOM && actual_format == 32 &&
      remaining == 0 && data) {
    const auto* values = reinterpret_cast<const Atom*>(data);
    atoms.assign(values, values + count);
  }
  if (data) {
    XFree(data);
  }
  return atoms;
}

bool ContainsAtom(const std::vector<Atom>& atoms, Atom expected) {
  return expected != None &&
         std::find(atoms.begin(), atoms.end(), expected) != atoms.end();
}

void ExpectPopup(Display* display, Window window) {
  const auto types = AtomProperty(display, window, "_NET_WM_WINDOW_TYPE");
  const Atom normal = XInternAtom(display, "_NET_WM_WINDOW_TYPE_NORMAL", True);
  if (!ContainsAtom(types, normal)) {
    throw std::runtime_error("window is not an EWMH normal window");
  }

  const auto states = AtomProperty(display, window, "_NET_WM_STATE");
  for (const char* name : {"_NET_WM_STATE_ABOVE",
                           "_NET_WM_STATE_SKIP_TASKBAR",
                           "_NET_WM_STATE_SKIP_PAGER"}) {
    if (!ContainsAtom(states, XInternAtom(display, name, True))) {
      throw std::runtime_error(std::string("window is missing EWMH state ") +
                               name);
    }
  }

  const Atom motif = XInternAtom(display, "_MOTIF_WM_HINTS", True);
  Atom actual_type = None;
  int actual_format = 0;
  unsigned long count = 0;
  unsigned long remaining = 0;
  unsigned char* data = nullptr;
  const int status =
      motif == None
          ? BadAtom
          : XGetWindowProperty(display, window, motif, 0, 5, False, motif,
                               &actual_type, &actual_format, &count, &remaining,
                               &data);
  const auto* hints = reinterpret_cast<const unsigned long*>(data);
  constexpr unsigned long kMotifDecorationsHint = 1UL << 1;
  const bool undecorated =
      status == Success && actual_type == motif && actual_format == 32 &&
      count >= 3 && remaining == 0 && data &&
      (hints[0] & kMotifDecorationsHint) != 0 && hints[2] == 0;
  if (data) {
    XFree(data);
  }
  if (!undecorated) {
    throw std::runtime_error("window is missing the undecorated Motif hint");
  }
  std::cout << "window has popup-like X11 properties\n";
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

bool ServerTimeIsAfter(Time candidate, Time previous) {
  const uint32_t candidate32 = static_cast<uint32_t>(candidate);
  const uint32_t previous32 = static_cast<uint32_t>(previous);
  if (candidate32 == static_cast<uint32_t>(CurrentTime)) {
    return false;
  }
  if (previous32 == static_cast<uint32_t>(CurrentTime)) {
    return true;
  }
  return static_cast<int32_t>(candidate32 - previous32) > 0;
}

void SelfTestServerClock() {
  constexpr uint32_t near_wrap = std::numeric_limits<uint32_t>::max() - 1;
  if (!ServerTimeIsAfter(1, CurrentTime) ||
      !ServerTimeIsAfter(100, 99) || ServerTimeIsAfter(99, 100) ||
      ServerTimeIsAfter(100, 100) ||
      !ServerTimeIsAfter(2, static_cast<Time>(near_wrap)) ||
      ServerTimeIsAfter(static_cast<Time>(near_wrap), 2) ||
      ServerTimeIsAfter(CurrentTime, static_cast<Time>(near_wrap))) {
    throw std::runtime_error("32-bit X11 server time ordering self-test failed");
  }
  std::cout << "X11 server clock wrap self-test passed\n";
}

class ServerClock {
 public:
  ServerClock(Display* display, Window root, Window target)
      : display_(display), target_(target) {
    pulse_atom_ = XInternAtom(display_, "_LYNX_LAUNCHER_TEST_TIME_PULSE", False);
    last_atom_ = XInternAtom(display_, "_LYNX_LAUNCHER_TEST_LAST_TIME", False);
    probe_ = XCreateSimpleWindow(display_, root, 0, 0, 1, 1, 0, 0, 0);
    if (pulse_atom_ == None || last_atom_ == None || probe_ == None) {
      throw std::runtime_error("could not initialize the X11 server clock");
    }
    XSelectInput(display_, probe_, PropertyChangeMask);

    Atom actual_type = None;
    int actual_format = 0;
    unsigned long count = 0;
    unsigned long remaining = 0;
    unsigned char* data = nullptr;
    const int status = XGetWindowProperty(
        display_, target_, last_atom_, 0, 1, False, XA_CARDINAL,
        &actual_type, &actual_format, &count, &remaining, &data);
    if (status == Success && actual_type == XA_CARDINAL &&
        actual_format == 32 && count == 1 && remaining == 0 && data) {
      last_ = *reinterpret_cast<unsigned long*>(data);
    }
    if (data) {
      XFree(data);
    }
  }

  ~ServerClock() {
    if (probe_ != None) {
      XDestroyWindow(display_, probe_);
    }
  }

  Time Next() {
    for (int attempt = 0; attempt < 2000; ++attempt) {
      const unsigned char pulse = static_cast<unsigned char>(attempt);
      XChangeProperty(display_, probe_, pulse_atom_, XA_INTEGER, 8,
                      PropModeReplace, &pulse, 1);
      XEvent event{};
      XWindowEvent(display_, probe_, PropertyChangeMask, &event);
      const Time candidate = event.xproperty.time;
      if (ServerTimeIsAfter(candidate, last_)) {
        last_ = candidate;
        const unsigned long value = last_;
        XChangeProperty(display_, target_, last_atom_, XA_CARDINAL, 32,
                        PropModeReplace,
                        reinterpret_cast<const unsigned char*>(&value), 1);
        XFlush(display_);
        return last_;
      }
      std::this_thread::sleep_for(std::chrono::milliseconds(1));
    }
    throw std::runtime_error(
        "X11 server time did not advance beyond the last injected key time");
  }

 private:
  Display* display_;
  Window target_;
  Window probe_ = None;
  Atom pulse_atom_ = None;
  Atom last_atom_ = None;
  Time last_ = CurrentTime;
};

void SendFocusOut(Display* display, Window window) {
  XEvent event{};
  event.xfocus.type = FocusOut;
  event.xfocus.display = display;
  event.xfocus.window = window;
  event.xfocus.mode = NotifyNormal;
  event.xfocus.detail = NotifyNonlinear;
  RequireSent(XSendEvent(display, window, False, FocusChangeMask, &event),
              "FocusOut");
  XSync(display, False);
}

void SendPointerPosition(Display* display, Window root, Window window,
                         int root_x, int root_y, int x, int y,
                         ServerClock& clock) {
  XEvent event{};
  event.xcrossing.type = EnterNotify;
  event.xcrossing.display = display;
  event.xcrossing.window = window;
  event.xcrossing.root = root;
  event.xcrossing.time = clock.Next();
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
  event.xmotion.time = clock.Next();
  event.xmotion.x = x;
  event.xmotion.y = y;
  event.xmotion.x_root = root_x;
  event.xmotion.y_root = root_y;
  event.xmotion.same_screen = True;
  RequireSent(XSendEvent(display, window, False, PointerMotionMask, &event),
              "MotionNotify");
}

void SendClick(Display* display, Window root, Window window, int root_x,
               int root_y, int x, int y, ServerClock& clock) {
  SendPointerPosition(display, root, window, root_x, root_y, x, y, clock);

  XEvent event{};
  event.xbutton.type = ButtonPress;
  event.xbutton.display = display;
  event.xbutton.window = window;
  event.xbutton.root = root;
  event.xbutton.time = clock.Next();
  event.xbutton.x = x;
  event.xbutton.y = y;
  event.xbutton.x_root = root_x;
  event.xbutton.y_root = root_y;
  event.xbutton.button = Button1;
  event.xbutton.same_screen = True;
  RequireSent(XSendEvent(display, window, False, ButtonPressMask, &event),
              "ButtonPress");

  event.xbutton.type = ButtonRelease;
  event.xbutton.time = clock.Next();
  event.xbutton.state = Button1Mask;
  RequireSent(XSendEvent(display, window, False, ButtonReleaseMask, &event),
              "ButtonRelease");
  XSync(display, False);
}

void SendText(Display* display, Window root, Window window,
              const std::string& text, ServerClock& clock) {
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
    event.xkey.time = clock.Next();
    event.xkey.state = shifted ? ShiftMask : 0;
    event.xkey.keycode = keycode;
    event.xkey.same_screen = True;
    RequireSent(XSendEvent(display, window, False, KeyPressMask, &event),
                "KeyPress");
    event.xkey.type = KeyRelease;
    event.xkey.time = clock.Next();
    RequireSent(XSendEvent(display, window, False, KeyReleaseMask, &event),
                "KeyRelease");
    XSync(display, False);
    std::this_thread::sleep_for(std::chrono::milliseconds(10));
  }
}

KeyCode TestKeyCode(Display* display, const std::string& key) {
  const KeySym symbol = key == "left-shift" ? XK_Shift_L : NoSymbol;
  const KeyCode keycode = XKeysymToKeycode(display, symbol);
  if (symbol == NoSymbol || keycode == 0) {
    throw std::runtime_error("the active X11 keymap cannot produce " + key);
  }
  return keycode;
}

void SendKey(Display* display, Window root, Window window, KeyCode keycode,
             int type, unsigned int state, ServerClock& clock,
             const char* event_name) {
  XEvent event{};
  event.xkey.type = type;
  event.xkey.display = display;
  event.xkey.window = window;
  event.xkey.root = root;
  event.xkey.time = clock.Next();
  event.xkey.state = state;
  event.xkey.keycode = keycode;
  event.xkey.same_screen = True;
  RequireSent(XSendEvent(display, window, False,
                         type == KeyPress ? KeyPressMask : KeyReleaseMask,
                         &event),
              event_name);
}

void SendRepeatKey(Display* display, Window root, Window window,
                   const std::string& key, ServerClock& clock) {
  const KeyCode keycode = TestKeyCode(display, key);
  SendKey(display, root, window, keycode, KeyPress, 0, clock, "KeyPress");
  SendKey(display, root, window, keycode, KeyPress, ShiftMask, clock,
          "repeat KeyPress");
  SendKey(display, root, window, keycode, KeyRelease, ShiftMask, clock,
          "KeyRelease");
  XSync(display, False);
}

void SendScroll(Display* display, Window root, Window window, int root_x,
                int root_y, int x, int y, bool up, ServerClock& clock) {
  SendPointerPosition(display, root, window, root_x, root_y, x, y, clock);
  XEvent event{};
  event.xbutton.type = ButtonPress;
  event.xbutton.display = display;
  event.xbutton.window = window;
  event.xbutton.root = root;
  event.xbutton.time = clock.Next();
  event.xbutton.x = x;
  event.xbutton.y = y;
  event.xbutton.x_root = root_x;
  event.xbutton.y_root = root_y;
  event.xbutton.button = up ? Button4 : Button5;
  event.xbutton.same_screen = True;
  RequireSent(XSendEvent(display, window, False, ButtonPressMask, &event),
              "scroll ButtonPress");
  event.xbutton.type = ButtonRelease;
  event.xbutton.time = clock.Next();
  RequireSent(XSendEvent(display, window, False, ButtonReleaseMask, &event),
              "scroll ButtonRelease");
  XSync(display, False);
}

void SendHeldInput(Display* display, Window root, Window window, int root_x,
                   int root_y, int x, int y, const std::string& key,
                   ServerClock& clock) {
  SendPointerPosition(display, root, window, root_x, root_y, x, y, clock);
  SendKey(display, root, window, TestKeyCode(display, key), KeyPress, 0, clock,
          "held KeyPress");

  XEvent event{};
  event.xbutton.type = ButtonPress;
  event.xbutton.display = display;
  event.xbutton.window = window;
  event.xbutton.root = root;
  event.xbutton.time = clock.Next();
  event.xbutton.x = x;
  event.xbutton.y = y;
  event.xbutton.x_root = root_x;
  event.xbutton.y_root = root_y;
  event.xbutton.button = Button1;
  event.xbutton.state = ShiftMask;
  event.xbutton.same_screen = True;
  RequireSent(XSendEvent(display, window, False, ButtonPressMask, &event),
              "held ButtonPress");
  XSync(display, False);
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

int x_get_image_error = Success;

int CaptureXError(Display*, XErrorEvent* event) {
  x_get_image_error = event->error_code;
  return 0;
}

int TryMatchingPixels(Display* display, Window window, const Options& options) {
  x_get_image_error = Success;
  auto previous_handler = XSetErrorHandler(CaptureXError);
  XImage* image =
      XGetImage(display, window, *options.x, *options.y, *options.width,
                *options.height, AllPlanes, ZPixmap);
  XSync(display, False);
  XSetErrorHandler(previous_handler);
  if (x_get_image_error != Success || !image) {
    if (image) {
      XDestroyImage(image);
    }
    return -1;
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

struct PpmImage {
  int width;
  int height;
  std::vector<uint8_t> pixels;
};

std::string ReadPpmToken(const std::vector<uint8_t>& bytes, size_t& offset) {
  while (offset < bytes.size()) {
    if (bytes[offset] == '#') {
      while (offset < bytes.size() && bytes[offset] != '\n') {
        ++offset;
      }
    } else if (std::isspace(bytes[offset])) {
      ++offset;
    } else {
      break;
    }
  }
  const size_t begin = offset;
  while (offset < bytes.size() && !std::isspace(bytes[offset]) &&
         bytes[offset] != '#') {
    ++offset;
  }
  if (begin == offset) {
    throw std::runtime_error("grim returned an invalid PPM header");
  }
  return std::string(reinterpret_cast<const char*>(bytes.data() + begin),
                     offset - begin);
}

PpmImage ParsePpm(const std::vector<uint8_t>& bytes) {
  size_t offset = 0;
  if (ReadPpmToken(bytes, offset) != "P6") {
    throw std::runtime_error("grim did not return a binary PPM image");
  }
  const int width = static_cast<int>(ParseUnsigned(
      ReadPpmToken(bytes, offset), "PPM width", std::numeric_limits<int>::max()));
  const int height = static_cast<int>(ParseUnsigned(
      ReadPpmToken(bytes, offset), "PPM height", std::numeric_limits<int>::max()));
  if (ReadPpmToken(bytes, offset) != "255" || width <= 0 || height <= 0 ||
      width > 32768 || height > 32768 ||
      offset >= bytes.size() || !std::isspace(bytes[offset])) {
    throw std::runtime_error("grim returned an unsupported PPM image");
  }
  if (bytes[offset++] == '\r' && offset < bytes.size() &&
      bytes[offset] == '\n') {
    ++offset;
  }
  const size_t pixel_bytes = static_cast<size_t>(width) * height * 3;
  if (pixel_bytes > bytes.size() - offset || bytes.size() - offset != pixel_bytes) {
    throw std::runtime_error("grim returned a truncated PPM image");
  }
  return {width, height,
          std::vector<uint8_t>(bytes.begin() + offset, bytes.end())};
}

int MatchingPpmPixels(const PpmImage& image, int x, int y, int width,
                      int height, const Options& options) {
  const int64_t right = static_cast<int64_t>(x) + width;
  const int64_t bottom = static_cast<int64_t>(y) + height;
  if (x < 0 || y < 0 || width <= 0 || height <= 0 ||
      right > image.width || bottom > image.height) {
    throw std::runtime_error("visible pixel region is outside the output");
  }
  int matches = 0;
  for (int image_y = y; image_y < bottom; ++image_y) {
    for (int image_x = x; image_x < right; ++image_x) {
      const size_t offset =
          (static_cast<size_t>(image_y) * image.width + image_x) * 3;
      if (std::abs(static_cast<int>(image.pixels[offset]) - *options.red) <=
              *options.tolerance &&
          std::abs(static_cast<int>(image.pixels[offset + 1]) -
                   *options.green) <= *options.tolerance &&
          std::abs(static_cast<int>(image.pixels[offset + 2]) - *options.blue) <=
              *options.tolerance) {
        ++matches;
      }
    }
  }
  return matches;
}

int MatchingScaledPpmPixels(const PpmImage& image, int x, int y, int width,
                            int height, int output_width, int output_height,
                            const Options& options) {
  if (output_width <= 0 || output_height <= 0) {
    throw std::runtime_error("scaled pixel output dimensions must be positive");
  }
  const int image_x = static_cast<int>(
      static_cast<int64_t>(x) * image.width / output_width);
  const int image_y = static_cast<int>(
      static_cast<int64_t>(y) * image.height / output_height);
  const int image_right = static_cast<int>(
      ((static_cast<int64_t>(x) + width) * image.width + output_width - 1) /
      output_width);
  const int image_bottom = static_cast<int>(
      ((static_cast<int64_t>(y) + height) * image.height + output_height - 1) /
      output_height);
  return MatchingPpmPixels(image, image_x, image_y, image_right - image_x,
                           image_bottom - image_y, options);
}

void ReapCaptureProcess(pid_t child, bool kill_child) {
  if (kill_child) {
    kill(child, SIGKILL);
  }
  while (waitpid(child, nullptr, 0) < 0 && errno == EINTR) {
  }
}

std::vector<uint8_t> CaptureProcess(const std::vector<std::string>& arguments,
                                    size_t size_limit, Deadline deadline,
                                    const char* failure_message) {
  if (arguments.empty()) {
    throw std::runtime_error("compositor capture command is empty");
  }
  int descriptors[2]{};
  if (pipe(descriptors) != 0) {
    throw std::runtime_error("could not create the compositor capture pipe");
  }
  const pid_t child = fork();
  if (child < 0) {
    close(descriptors[0]);
    close(descriptors[1]);
    throw std::runtime_error("could not start the compositor capture");
  }
  if (child == 0) {
    close(descriptors[0]);
    if (dup2(descriptors[1], STDOUT_FILENO) < 0) {
      _exit(126);
    }
    close(descriptors[1]);
    std::vector<char*> argv;
    argv.reserve(arguments.size() + 1);
    for (const std::string& argument : arguments) {
      argv.push_back(const_cast<char*>(argument.c_str()));
    }
    argv.push_back(nullptr);
    execvp(argv.front(), argv.data());
    _exit(127);
  }

  close(descriptors[1]);
  const int flags = fcntl(descriptors[0], F_GETFL, 0);
  if (flags < 0 || fcntl(descriptors[0], F_SETFL, flags | O_NONBLOCK) != 0) {
    close(descriptors[0]);
    ReapCaptureProcess(child, true);
    throw std::runtime_error("could not make compositor capture nonblocking");
  }
  std::vector<uint8_t> bytes;
  uint8_t buffer[64 * 1024];
  bool pipe_closed = false;
  bool child_exited = false;
  int status = 0;
  for (;;) {
    for (;;) {
      const ssize_t count = read(descriptors[0], buffer, sizeof(buffer));
      if (count > 0) {
        if (static_cast<size_t>(count) > size_limit - bytes.size()) {
          close(descriptors[0]);
          ReapCaptureProcess(child, !child_exited);
          throw std::runtime_error("compositor capture exceeded the size limit");
        }
        bytes.insert(bytes.end(), buffer, buffer + count);
        continue;
      }
      if (count == 0) {
        pipe_closed = true;
      } else if (errno != EAGAIN && errno != EWOULDBLOCK && errno != EINTR) {
        close(descriptors[0]);
        ReapCaptureProcess(child, !child_exited);
        throw std::runtime_error("could not read the compositor capture");
      }
      break;
    }

    if (!child_exited) {
      const pid_t waited = waitpid(child, &status, WNOHANG);
      if (waited == child) {
        child_exited = true;
      } else if (waited < 0 && errno != EINTR) {
        close(descriptors[0]);
        ReapCaptureProcess(child, true);
        throw std::runtime_error("could not wait for the compositor capture");
      }
    }
    if (pipe_closed && child_exited) {
      break;
    }
    const auto now = std::chrono::steady_clock::now();
    if (now >= deadline) {
      close(descriptors[0]);
      ReapCaptureProcess(child, !child_exited);
      throw std::runtime_error("compositor capture exceeded its caller deadline");
    }
    const auto remaining = std::chrono::duration_cast<std::chrono::milliseconds>(
        deadline - now);
    const int timeout = static_cast<int>(std::clamp<int64_t>(
        remaining.count(), 1, std::numeric_limits<int>::max()));
    pollfd descriptor{descriptors[0], POLLIN | POLLHUP, 0};
    const int poll_result =
        pipe_closed ? poll(nullptr, 0, std::min(timeout, 10))
                    : poll(&descriptor, 1, timeout);
    if (poll_result < 0 && errno != EINTR) {
      close(descriptors[0]);
      ReapCaptureProcess(child, !child_exited);
      throw std::runtime_error("could not poll the compositor capture");
    }
  }
  close(descriptors[0]);
  if (!WIFEXITED(status) || WEXITSTATUS(status) != 0) {
    throw std::runtime_error(failure_message);
  }
  return bytes;
}

PpmImage CaptureOutput(const std::string& output, Deadline deadline) {
  return ParsePpm(CaptureProcess({"grim", "-t", "ppm", "-o", output, "-"},
                                  128 * 1024 * 1024, deadline,
                                  "grim could not capture the compositor output"));
}

size_t JsonValueOffset(const std::string& json, const std::string& key) {
  const std::string needle = "\"" + key + "\":";
  size_t offset = json.find(needle);
  if (offset == std::string::npos) {
    throw std::runtime_error("niri response is missing " + key);
  }
  offset += needle.size();
  while (offset < json.size() && std::isspace(
                                     static_cast<unsigned char>(json[offset]))) {
    ++offset;
  }
  return offset;
}

std::string JsonString(const std::string& json, const std::string& key) {
  size_t offset = JsonValueOffset(json, key);
  if (offset >= json.size() || json[offset++] != '"') {
    throw std::runtime_error("niri response has an invalid " + key);
  }
  const size_t end = json.find('"', offset);
  if (end == std::string::npos || json.find('\\', offset) < end) {
    throw std::runtime_error("niri response has an unsupported " + key);
  }
  return json.substr(offset, end - offset);
}

double JsonNumber(const std::string& json, const std::string& key) {
  const size_t offset = JsonValueOffset(json, key);
  char* end = nullptr;
  errno = 0;
  const double value = std::strtod(json.c_str() + offset, &end);
  if (errno != 0 || end == json.c_str() + offset || !std::isfinite(value)) {
    throw std::runtime_error("niri response has an invalid " + key);
  }
  return value;
}

std::pair<double, double> JsonNumberPair(const std::string& json,
                                         const std::string& key) {
  size_t offset = JsonValueOffset(json, key);
  if (offset >= json.size() || json[offset++] != '[') {
    throw std::runtime_error("niri response has an invalid " + key);
  }
  char* end = nullptr;
  errno = 0;
  const double first = std::strtod(json.c_str() + offset, &end);
  if (errno != 0 || end == json.c_str() + offset || *end != ',') {
    throw std::runtime_error("niri response has an invalid " + key);
  }
  offset = static_cast<size_t>(end - json.c_str()) + 1;
  errno = 0;
  const double second = std::strtod(json.c_str() + offset, &end);
  if (errno != 0 || end == json.c_str() + offset || *end != ']' ||
      !std::isfinite(first) || !std::isfinite(second)) {
    throw std::runtime_error("niri response has an invalid " + key);
  }
  return {first, second};
}

bool JsonBoolean(const std::string& json, const std::string& key) {
  const size_t offset = JsonValueOffset(json, key);
  if (json.compare(offset, 4, "true") == 0) {
    return true;
  }
  if (json.compare(offset, 5, "false") == 0) {
    return false;
  }
  throw std::runtime_error("niri response has an invalid " + key);
}

std::string CaptureNiriJson(const std::string& subject, Deadline deadline) {
  const std::vector<uint8_t> bytes = CaptureProcess(
      {"niri", "msg", "--json", subject}, 1024 * 1024, deadline,
      "niri could not report compositor geometry");
  return std::string(bytes.begin(), bytes.end());
}

int CheckedRoundedInt(double value, const std::string& label) {
  if (!std::isfinite(value) || value < std::numeric_limits<int>::min() ||
      value > std::numeric_limits<int>::max()) {
    throw std::runtime_error(label + " is outside the supported integer range");
  }
  const long rounded = std::lround(value);
  if (rounded < std::numeric_limits<int>::min() ||
      rounded > std::numeric_limits<int>::max()) {
    throw std::runtime_error(label + " rounds outside the supported integer range");
  }
  return static_cast<int>(rounded);
}

int CheckedCompositorCoordinate(double value, const std::string& label) {
  if (!std::isfinite(value) || std::abs(value) > kMaximumCompositorCoordinate) {
    throw std::runtime_error(label + " is outside the supported compositor range");
  }
  return CheckedRoundedInt(value, label);
}

int CheckedCompositorDimension(double value, const std::string& label) {
  if (!std::isfinite(value) || value <= 0 ||
      value > kMaximumCompositorDimension) {
    throw std::runtime_error(label + " is outside the supported compositor range");
  }
  return CheckedRoundedInt(value, label);
}

int CheckedAdd(int left, int right, const std::string& label) {
  const int64_t result = static_cast<int64_t>(left) + right;
  if (result < std::numeric_limits<int>::min() ||
      result > std::numeric_limits<int>::max()) {
    throw std::runtime_error(label + " overflows the supported integer range");
  }
  return static_cast<int>(result);
}

std::optional<int> NiriMatchingPixels(const XWindowAttributes& attributes,
                                      const Options& options, Deadline deadline) {
  if (!std::getenv("NIRI_SOCKET")) {
    return std::nullopt;
  }
  const std::string window = CaptureNiriJson("focused-window", deadline);
  if (!JsonBoolean(window, "is_focused") ||
      !JsonBoolean(window, "is_floating")) {
    throw std::runtime_error(
        "niri pixel assertion target is not the focused floating window");
  }
  const auto position = JsonNumberPair(window, "tile_pos_in_workspace_view");
  const auto window_size = JsonNumberPair(window, "window_size");

  const std::string output = CaptureNiriJson("focused-output", deadline);
  const size_t logical_offset = output.find("\"logical\":{");
  if (logical_offset == std::string::npos) {
    throw std::runtime_error("niri response is missing logical output geometry");
  }
  const std::string logical = output.substr(logical_offset);
  const double scale = JsonNumber(logical, "scale");
  if (scale <= 0 || scale > kMaximumScale ||
      JsonString(logical, "transform") != "Normal" ||
      CheckedCompositorDimension(window_size.first * scale, "niri window width") !=
          attributes.width ||
      CheckedCompositorDimension(window_size.second * scale, "niri window height") !=
          attributes.height) {
    throw std::runtime_error(
        "niri compositor geometry does not match the XWayland window");
  }

  const PpmImage image = CaptureOutput(JsonString(output, "name"), deadline);
  if (CheckedCompositorDimension(JsonNumber(logical, "width") * scale,
                                 "niri output width") != image.width ||
      CheckedCompositorDimension(JsonNumber(logical, "height") * scale,
                                 "niri output height") != image.height) {
    throw std::runtime_error(
        "niri output geometry does not match the compositor capture");
  }
  const int image_x = CheckedAdd(
      CheckedCompositorCoordinate(position.first * scale, "niri window x"), *options.x,
      "niri capture x");
  const int image_y = CheckedAdd(
      CheckedCompositorCoordinate(position.second * scale, "niri window y"), *options.y,
      "niri capture y");
  return MatchingPpmPixels(image, image_x, image_y, *options.width,
                           *options.height, options);
}

int VisibleMatchingPixels(Display* display, Window root, Window window,
                          const Options& options, Deadline deadline) {
  XWindowAttributes attributes{};
  if (!XGetWindowAttributes(display, window, &attributes) ||
      attributes.c_class != InputOutput || attributes.map_state != IsViewable ||
      *options.x < 0 || *options.y < 0 ||
      *options.x + *options.width > attributes.width ||
      *options.y + *options.height > attributes.height) {
    throw std::runtime_error(
        "pixel assertion target is not a visible in-bounds window");
  }
  const Atom above = XInternAtom(display, "_NET_WM_STATE_ABOVE", True);
  if (!ContainsAtom(AtomProperty(display, window, "_NET_WM_STATE"), above)) {
    throw std::runtime_error(
        "pixel assertion target is not an unobscured launcher popup");
  }
  Window focused = None;
  int revert_to = RevertToNone;
  XGetInputFocus(display, &focused, &revert_to);
  bool owns_focus = false;
  for (Window current = focused; current != None && current != PointerRoot;) {
    if (current == window) {
      owns_focus = true;
      break;
    }
    Window query_root = None;
    Window parent = None;
    Window* children = nullptr;
    unsigned int child_count = 0;
    if (!XQueryTree(display, current, &query_root, &parent, &children,
                    &child_count)) {
      break;
    }
    if (children) {
      XFree(children);
    }
    current = parent;
  }
  if (!owns_focus) {
    throw std::runtime_error(
        "pixel assertion target is not the active unobscured popup");
  }
  Window translated_child = None;
  int root_x = 0;
  int root_y = 0;
  if (!XTranslateCoordinates(display, window, root, 0, 0, &root_x, &root_y,
                             &translated_child)) {
    throw std::runtime_error("could not translate window coordinates to root");
  }
  Options root_options = options;
  root_options.x = root_x + *options.x;
  root_options.y = root_y + *options.y;
  const int root_matches = TryMatchingPixels(display, root, root_options);
  if (root_matches >= 0) {
    return root_matches;
  }
  if (const std::optional<int> niri_matches =
          NiriMatchingPixels(attributes, options, deadline)) {
    return *niri_matches;
  }

  throw std::runtime_error(
      "X11 root readback is unavailable; compositor capture is supported only "
      "for niri/XWayland with niri and grim on PATH");
}

void SelfTestVisiblePixels() {
  const std::vector<uint8_t> bytes = {'P', '6', '\n', '2', ' ', '1', '\n',
                                      '2', '5', '5', '\n', 0,   0,   0,
                                      255, 107, 61};
  const PpmImage image = ParsePpm(bytes);
  Options options;
  options.width = 2;
  options.height = 1;
  options.red = 255;
  options.green = 107;
  options.blue = 61;
  options.tolerance = 0;
  if (image.width != 2 || image.height != 1 ||
      MatchingPpmPixels(image, 0, 0, 2, 1, options) != 1 ||
      MatchingScaledPpmPixels(image, 0, 0, 1, 1, 1, 1, options) != 1) {
    throw std::runtime_error("visible pixel PPM self-test failed");
  }
  const std::string window =
      R"({"is_focused":true,"is_floating":true,"window_size":[700,475],"tile_pos_in_workspace_view":[260.0,81.0]})";
  const auto position = JsonNumberPair(window, "tile_pos_in_workspace_view");
  const auto size = JsonNumberPair(window, "window_size");
  const std::string output =
      R"({"name":"DP-1","logical":{"width":1920,"height":1080,"scale":2.0,"transform":"Normal"}})";
  const std::string logical = output.substr(output.find("\"logical\":{") + 10);
  if (!JsonBoolean(window, "is_focused") ||
      !JsonBoolean(window, "is_floating") || position.first != 260.0 ||
      position.second != 81.0 || size.first != 700.0 ||
      size.second != 475.0 || JsonString(output, "name") != "DP-1" ||
      JsonNumber(logical, "scale") != 2.0 ||
      JsonString(logical, "transform") != "Normal") {
    throw std::runtime_error("visible pixel niri geometry self-test failed");
  }
  const auto deadline = std::chrono::steady_clock::now() +
                        std::chrono::seconds(1);
  const std::vector<uint8_t> captured = CaptureProcess(
      {"/bin/echo", "capture-ok"}, 64, deadline,
      "capture process self-test failed");
  if (std::string(captured.begin(), captured.end()) != "capture-ok\n") {
    throw std::runtime_error("capture process output self-test failed");
  }
  bool timed_out = false;
  try {
    CaptureProcess({"/bin/sleep", "1"}, 64,
                   std::chrono::steady_clock::now() +
                       std::chrono::milliseconds(10),
                   "capture process timeout self-test failed");
  } catch (const std::runtime_error& error) {
    timed_out = std::string(error.what()).find("deadline") != std::string::npos;
  }
  if (!timed_out) {
    throw std::runtime_error("capture process deadline self-test failed");
  }
  bool size_limited = false;
  try {
    CaptureProcess({"/bin/echo", "too-large"}, 1, deadline,
                   "capture process size self-test failed");
  } catch (const std::runtime_error& error) {
    size_limited =
        std::string(error.what()).find("size limit") != std::string::npos;
  }
  if (!size_limited) {
    throw std::runtime_error("capture process size-limit self-test failed");
  }
  bool rejected_geometry = false;
  try {
    CheckedRoundedInt(std::numeric_limits<double>::infinity(),
                      "self-test geometry");
  } catch (const std::runtime_error&) {
    rejected_geometry = true;
  }
  if (!rejected_geometry) {
    throw std::runtime_error("niri non-finite geometry self-test failed");
  }
}

void ExpectPixel(Display* display, Window root, Window window,
                 const Options& options) {
  const auto deadline = std::chrono::steady_clock::now() +
                        std::chrono::milliseconds(*options.timeout_ms);
  int best_match_count = 0;
  do {
    best_match_count = std::max(
        best_match_count,
        VisibleMatchingPixels(display, root, window, options, deadline));
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

void ExpectNoPixel(Display* display, Window root, Window window,
                   const Options& options) {
  const auto deadline = std::chrono::steady_clock::now() +
                        std::chrono::milliseconds(*options.timeout_ms);
  int lowest_match_count = std::numeric_limits<int>::max();
  do {
    lowest_match_count = std::min(
        lowest_match_count,
        VisibleMatchingPixels(display, root, window, options, deadline));
    if (lowest_match_count <= *options.maximum_matches) {
      std::cout << "found " << lowest_match_count
                << " rendered pixels near excluded rgb(" << *options.red << ','
                << *options.green << ',' << *options.blue << ")\n";
      return;
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(50));
  } while (std::chrono::steady_clock::now() < deadline);
  throw std::runtime_error(
      "rendered exclusion assertion found " +
      std::to_string(lowest_match_count) + ", expected at most " +
      std::to_string(*options.maximum_matches));
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
    if (argc == 2 && std::string(argv[1]) == "--self-test-server-clock") {
      SelfTestServerClock();
      return 0;
    }
    if (argc == 2 && std::string(argv[1]) == "--self-test-visible-pixels") {
      SelfTestVisiblePixels();
      return 0;
    }
    const Options options = ParseOptions(argc, argv);
    const bool pixel_action = options.action == Action::kExpectPixel ||
                              options.action == Action::kExpectNoPixel;
    if (pixel_action && std::getenv("NIRI_SOCKET") &&
        (!ExecutableOnPath("niri") || !ExecutableOnPath("grim"))) {
      throw std::runtime_error(
          "niri/XWayland visible-pixel capture requires niri and grim on PATH");
    }
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

    if (options.action == Action::kExpectPopup) {
      ExpectPopup(display, window);
      return 0;
    }
    if (options.action == Action::kDefocus) {
      SendFocusOut(display, window);
      return 0;
    }
    if (options.action == Action::kType) {
      ServerClock clock(display, root, window);
      SendText(display, root, window, *options.text, clock);
      return 0;
    }
    if (options.action == Action::kRepeatKey) {
      ServerClock clock(display, root, window);
      SendRepeatKey(display, root, window, *options.key, clock);
      return 0;
    }

    Options physical = options;
    const double inferred_scale = std::min(
        static_cast<double>(attributes.width) / kLauncherLogicalWidth,
        static_cast<double>(attributes.height) / kLauncherLogicalHeight);
    const double scale = options.scale.value_or(
        XSettingsScale(display).value_or(inferred_scale));
    if (!std::isfinite(scale) || scale <= 0 || scale > kMaximumScale) {
      throw std::runtime_error(
          "effective X11 scale must be finite and greater than 0 up to 8");
    }
    const double scale_x = scale;
    const double scale_y = scale;
    physical.x = CheckedRoundedInt(*options.x * scale_x, "scaled x");
    physical.y = CheckedRoundedInt(*options.y * scale_y, "scaled y");
    if (options.other_x) {
      physical.other_x =
          CheckedRoundedInt(*options.other_x * scale_x, "scaled other x");
      physical.other_y =
          CheckedRoundedInt(*options.other_y * scale_y, "scaled other y");
    }
    if (*physical.x >= attributes.width || *physical.y >= attributes.height) {
      throw std::runtime_error(
          "coordinates are outside the specified PID window");
    }
    if (options.action == Action::kExpectPixel ||
        options.action == Action::kExpectNoPixel ||
        options.action == Action::kExpectRegionsDiffer) {
      physical.width = std::max(
          1, CheckedRoundedInt(*options.width * scale_x, "scaled width"));
      physical.height = std::max(
          1, CheckedRoundedInt(*options.height * scale_y, "scaled height"));
      if (options.minimum_matches) {
        physical.minimum_matches = std::max(
            1, CheckedRoundedInt(*options.minimum_matches * scale_x * scale_y,
                                 "scaled minimum matches"));
      }
      if (options.maximum_matches) {
        physical.maximum_matches = std::max(
            0, CheckedRoundedInt(*options.maximum_matches * scale_x * scale_y,
                                 "scaled maximum matches"));
      }
      if (options.minimum_differences) {
        physical.minimum_differences = std::max(
            1, CheckedRoundedInt(*options.minimum_differences * scale_x * scale_y,
                                 "scaled minimum differences"));
      }
      if (*physical.width > attributes.width - *physical.x ||
          *physical.height > attributes.height - *physical.y) {
        throw std::runtime_error(
            "pixel region is outside the specified PID window");
      }
      if (options.action == Action::kExpectPixel) {
        ExpectPixel(display, root, window, physical);
        return 0;
      }
      if (options.action == Action::kExpectNoPixel) {
        ExpectNoPixel(display, root, window, physical);
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
    ServerClock clock(display, root, window);
    if (options.action == Action::kScroll) {
      SendScroll(display, root, window, window_root_x + *physical.x,
                 window_root_y + *physical.y, *physical.x, *physical.y,
                 *options.direction == "up", clock);
    } else if (options.action == Action::kHoldInput) {
      SendHeldInput(display, root, window, window_root_x + *physical.x,
                    window_root_y + *physical.y, *physical.x, *physical.y,
                    *options.key, clock);
    } else {
      SendClick(display, root, window, window_root_x + *physical.x,
                window_root_y + *physical.y, *physical.x, *physical.y, clock);
    }
    return 0;
  } catch (const std::exception& error) {
    std::cerr << "x11_click_test_driver: " << error.what() << '\n';
    return 1;
  }
}
