cmake_minimum_required(VERSION 3.16)
project(lynx_launcher_host LANGUAGES C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

include(CTest)
find_package(OpenGL REQUIRED)
find_package(Threads REQUIRED)
find_package(X11 REQUIRED)

get_filename_component(LAUNCHER_ROOT "${CMAKE_CURRENT_LIST_DIR}/.." ABSOLUTE)
set(LYNX_SDK_DIR "" CACHE PATH "Verified extracted Lynx Linux SDK directory")
if(NOT LYNX_SDK_DIR)
  message(FATAL_ERROR
    "LYNX_SDK_DIR must point to an SDK prepared by scripts/bootstrap.sh")
endif()
set(GLFW_SOURCE_DIR "${LAUNCHER_ROOT}/third_party/lynx/third_party/glfw"
    CACHE PATH "Vendored GLFW source directory")
set(PLATFORM_DIR "${LAUNCHER_ROOT}/platform"
    CACHE PATH "Rust platform crate directory")

foreach(required_path
        "${LYNX_SDK_DIR}/lib/liblynx.so"
        "${LYNX_SDK_DIR}/include"
        "${LYNX_SDK_DIR}/data/icudtl.dat"
        "${LYNX_SDK_DIR}/lynx_core.js"
        "${GLFW_SOURCE_DIR}/CMakeLists.txt"
        "${PLATFORM_DIR}/Cargo.toml"
        "${PLATFORM_DIR}/include/lynx_launcher.h"
        "${LAUNCHER_ROOT}/rust-toolchain.toml"
        "${LAUNCHER_ROOT}/ui/dist/main.lynx.bundle")
  if(NOT EXISTS "${required_path}")
    message(FATAL_ERROR "Required launcher dependency is missing: ${required_path}")
  endif()
endforeach()

set(GLFW_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(GLFW_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(GLFW_BUILD_DOCS OFF CACHE BOOL "" FORCE)
set(GLFW_INSTALL OFF CACHE BOOL "" FORCE)
set(GLFW_USE_WAYLAND OFF CACHE BOOL "" FORCE)
# GLFW 3.3 declares compatibility with CMake 3.0; CMake 4 needs this floor.
set(CMAKE_POLICY_VERSION_MINIMUM 3.5)
add_subdirectory("${GLFW_SOURCE_DIR}" "${CMAKE_BINARY_DIR}/glfw" EXCLUDE_FROM_ALL)

add_library(lynx_sdk SHARED IMPORTED GLOBAL)
set_target_properties(lynx_sdk PROPERTIES
  IMPORTED_LOCATION "${LYNX_SDK_DIR}/lib/liblynx.so"
)

set(CARGO_TARGET_DIR "${CMAKE_BINARY_DIR}/cargo-target")
set(PLATFORM_STATIC_LIB "${CARGO_TARGET_DIR}/release/liblynx_launcher_platform.a")
file(GLOB_RECURSE PLATFORM_RUST_SOURCES CONFIGURE_DEPENDS
  "${PLATFORM_DIR}/src/*.rs"
)
add_custom_command(
  OUTPUT "${PLATFORM_STATIC_LIB}"
  COMMAND "${CMAKE_COMMAND}" -E env
          "CARGO_TARGET_DIR=${CARGO_TARGET_DIR}"
          cargo build --release --locked
  WORKING_DIRECTORY "${PLATFORM_DIR}"
  DEPENDS
    "${PLATFORM_DIR}/Cargo.toml"
    "${PLATFORM_DIR}/Cargo.lock"
    "${LAUNCHER_ROOT}/rust-toolchain.toml"
    ${PLATFORM_RUST_SOURCES}
  COMMENT "Building the Rust launcher platform static library"
  VERBATIM
)
add_custom_target(platform_staticlib DEPENDS "${PLATFORM_STATIC_LIB}")
add_library(launcher_platform STATIC IMPORTED GLOBAL)
set_target_properties(launcher_platform PROPERTIES
  IMPORTED_LOCATION "${PLATFORM_STATIC_LIB}"
)

add_library(host_support STATIC
  src/support.cc
)
target_include_directories(host_support PUBLIC "${CMAKE_CURRENT_LIST_DIR}/src")

add_executable(lynx-launcher
  src/main.cc
)
add_dependencies(lynx-launcher platform_staticlib)
target_compile_definitions(lynx-launcher PRIVATE
  USE_WEAK_SUFFIX_NAPI=1
)
target_include_directories(lynx-launcher PRIVATE
  "${LYNX_SDK_DIR}/include"
  "${PLATFORM_DIR}/include"
)
target_link_libraries(lynx-launcher PRIVATE
  host_support
  lynx_sdk
  launcher_platform
  glfw
  OpenGL::GL
  Threads::Threads
  ${CMAKE_DL_LIBS}
  m
)
set_target_properties(lynx-launcher PROPERTIES
  BUILD_RPATH "$ORIGIN"
  BUILD_WITH_INSTALL_RPATH TRUE
  INSTALL_RPATH "$ORIGIN"
  INSTALL_RPATH_USE_LINK_PATH FALSE
)

set(RUNTIME_LIBLYNX "${CMAKE_BINARY_DIR}/liblynx.so")
set(RUNTIME_ICU "${CMAKE_BINARY_DIR}/resources/icudtl.dat")
set(RUNTIME_CORE "${CMAKE_BINARY_DIR}/resources/lynx_core.js")
set(RUNTIME_DEFAULT_CORE "${CMAKE_BINARY_DIR}/lynx_core.js")
set(RUNTIME_BUNDLE "${CMAKE_BINARY_DIR}/resources/main.lynx.bundle")
set(REFRESH_RESOURCE_SCRIPT
    "${CMAKE_CURRENT_LIST_DIR}/cmake/refresh_resource.cmake")
add_custom_target(runtime_resources ALL
  COMMAND "${CMAKE_COMMAND}"
          "-DSOURCE=${LYNX_SDK_DIR}/lib/liblynx.so"
          "-DDESTINATION=${RUNTIME_LIBLYNX}"
          -P "${REFRESH_RESOURCE_SCRIPT}"
  COMMAND "${CMAKE_COMMAND}"
          "-DSOURCE=${LYNX_SDK_DIR}/data/icudtl.dat"
          "-DDESTINATION=${RUNTIME_ICU}"
          -P "${REFRESH_RESOURCE_SCRIPT}"
  COMMAND "${CMAKE_COMMAND}"
          "-DSOURCE=${LYNX_SDK_DIR}/lynx_core.js"
          "-DDESTINATION=${RUNTIME_CORE}"
          -P "${REFRESH_RESOURCE_SCRIPT}"
  COMMAND "${CMAKE_COMMAND}"
          "-DSOURCE=${LYNX_SDK_DIR}/lynx_core.js"
          "-DDESTINATION=${RUNTIME_DEFAULT_CORE}"
          -P "${REFRESH_RESOURCE_SCRIPT}"
  COMMAND "${CMAKE_COMMAND}"
          "-DSOURCE=${LAUNCHER_ROOT}/ui/dist/main.lynx.bundle"
          "-DDESTINATION=${RUNTIME_BUNDLE}"
          -P "${REFRESH_RESOURCE_SCRIPT}"
  BYPRODUCTS
  "${RUNTIME_LIBLYNX}"
  "${RUNTIME_ICU}"
  "${RUNTIME_CORE}"
  "${RUNTIME_DEFAULT_CORE}"
  "${RUNTIME_BUNDLE}"
  DEPENDS
  "${LYNX_SDK_DIR}/lib/liblynx.so"
  "${LYNX_SDK_DIR}/data/icudtl.dat"
  "${LYNX_SDK_DIR}/lynx_core.js"
  "${LAUNCHER_ROOT}/ui/dist/main.lynx.bundle"
  "${REFRESH_RESOURCE_SCRIPT}"
  COMMENT "Refreshing launcher runtime resources by content"
  VERBATIM
)
add_dependencies(lynx-launcher runtime_resources)

if(BUILD_TESTING)
  add_executable(host_support_test
    tests/support_test.cc
  )
  target_link_libraries(host_support_test PRIVATE host_support)

  add_executable(x11_click_test_driver
    tests/x11_click_test_driver.cc
  )
  target_link_libraries(x11_click_test_driver PRIVATE
    X11::X11
  )

  add_test(NAME host_support_test COMMAND host_support_test)
  add_test(NAME launcher_resource_smoke
           COMMAND lynx-launcher --check-resources)
  add_test(NAME launcher_rejects_empty_icu
    COMMAND "${CMAKE_COMMAND}"
            "-DLAUNCHER=$<TARGET_FILE:lynx-launcher>"
            "-DTEST_ROOT=${CMAKE_CURRENT_BINARY_DIR}/empty-icu-test"
            -P "${CMAKE_CURRENT_LIST_DIR}/cmake/test_empty_icu.cmake")
  add_test(NAME runtime_resource_refresh_older_source
    COMMAND "${CMAKE_COMMAND}"
            "-DREFRESH_SCRIPT=${REFRESH_RESOURCE_SCRIPT}"
            "-DTEST_ROOT=${CMAKE_CURRENT_BINARY_DIR}/resource-refresh-test"
            -P "${CMAKE_CURRENT_LIST_DIR}/cmake/test_resource_refresh.cmake")
endif()
