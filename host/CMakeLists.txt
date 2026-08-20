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
set(HOST_RS_DIR "${LAUNCHER_ROOT}/host-rs"
    CACHE PATH "Rust host tracer crate directory")
set(LYNX_SYS_DIR "${LAUNCHER_ROOT}/lynx-sys"
    CACHE PATH "Rust Lynx bindings crate directory")

foreach(required_path
        "${LYNX_SDK_DIR}/lib/liblynx.so"
        "${LYNX_SDK_DIR}/include"
        "${LYNX_SDK_DIR}/data/icudtl.dat"
        "${LYNX_SDK_DIR}/lynx_core.js"
        "${GLFW_SOURCE_DIR}/CMakeLists.txt"
        "${LAUNCHER_ROOT}/Cargo.toml"
        "${LAUNCHER_ROOT}/Cargo.lock"
        "${PLATFORM_DIR}/Cargo.toml"
        "${HOST_RS_DIR}/Cargo.toml"
        "${LYNX_SYS_DIR}/Cargo.toml"
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
set(BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
# GLFW 3.3 declares compatibility with CMake 3.0; CMake 4 needs this floor.
set(CMAKE_POLICY_VERSION_MINIMUM 3.5)
add_subdirectory("${GLFW_SOURCE_DIR}" "${CMAKE_BINARY_DIR}/glfw" EXCLUDE_FROM_ALL)

add_library(lynx_sdk SHARED IMPORTED GLOBAL)
set_target_properties(lynx_sdk PROPERTIES
  IMPORTED_LOCATION "${LYNX_SDK_DIR}/lib/liblynx.so"
)

add_library(lynx_sys_shim STATIC
  "${LYNX_SYS_DIR}/src/lynx_sys_shim.cc"
)
target_include_directories(lynx_sys_shim
  PUBLIC "${LYNX_SYS_DIR}/include"
  PRIVATE "${LYNX_SDK_DIR}/include"
)
target_link_libraries(lynx_sys_shim PRIVATE lynx_sdk)

set(CARGO_TARGET_DIR "${CMAKE_BINARY_DIR}/cargo-target")
set(PLATFORM_STATIC_LIB "${CARGO_TARGET_DIR}/release/liblynx_launcher_platform.a")
set(RUST_HOST_CARGO_BINARY "${CARGO_TARGET_DIR}/release/lynx-launcher-rs")
set(RUST_HOST_BINARY "${CMAKE_BINARY_DIR}/lynx-launcher-rs")
file(GLOB_RECURSE PLATFORM_RUST_SOURCES CONFIGURE_DEPENDS
  "${PLATFORM_DIR}/src/*.rs"
)
file(GLOB_RECURSE HOST_RS_RUST_SOURCES CONFIGURE_DEPENDS
  "${HOST_RS_DIR}/src/*.rs"
)
file(GLOB_RECURSE LYNX_SYS_RUST_SOURCES CONFIGURE_DEPENDS
  "${LYNX_SYS_DIR}/src/*.rs"
)
add_custom_command(
  OUTPUT "${PLATFORM_STATIC_LIB}"
  COMMAND "${CMAKE_COMMAND}" -E env
          "CARGO_TARGET_DIR=${CARGO_TARGET_DIR}"
          cargo build --release --locked -p lynx-launcher-platform
  WORKING_DIRECTORY "${LAUNCHER_ROOT}"
  DEPENDS
    "${LAUNCHER_ROOT}/Cargo.toml"
    "${LAUNCHER_ROOT}/Cargo.lock"
    "${PLATFORM_DIR}/Cargo.toml"
    "${LAUNCHER_ROOT}/rust-toolchain.toml"
    ${PLATFORM_RUST_SOURCES}
  COMMENT "Building the Rust launcher platform static library"
  VERBATIM
)
add_custom_command(
  OUTPUT "${RUST_HOST_CARGO_BINARY}"
  COMMAND "${CMAKE_COMMAND}" -E env
           "CARGO_TARGET_DIR=${CARGO_TARGET_DIR}"
           "LYNX_SDK_DIR=${LYNX_SDK_DIR}"
           "LYNX_LAUNCHER_LYNX_SHIM_ARCHIVE=$<TARGET_FILE:lynx_sys_shim>"
           "LYNX_LAUNCHER_GLFW_ARCHIVE=$<TARGET_FILE:glfw>"
           "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=${CMAKE_CXX_COMPILER}"
           cargo build --release --locked -p lynx-launcher-host
          --bin lynx-launcher-rs
  WORKING_DIRECTORY "${LAUNCHER_ROOT}"
  DEPENDS
    glfw
    lynx_sys_shim
    "${PLATFORM_STATIC_LIB}"
    "${LAUNCHER_ROOT}/Cargo.toml"
    "${LAUNCHER_ROOT}/Cargo.lock"
    "${HOST_RS_DIR}/Cargo.toml"
    "${HOST_RS_DIR}/build.rs"
    "${LYNX_SYS_DIR}/Cargo.toml"
    "${LYNX_SYS_DIR}/build.rs"
    "${LAUNCHER_ROOT}/rust-toolchain.toml"
    ${HOST_RS_RUST_SOURCES}
    ${LYNX_SYS_RUST_SOURCES}
  COMMENT "Building the Rust host tracer"
  VERBATIM
)
add_custom_command(
  OUTPUT "${RUST_HOST_BINARY}"
  COMMAND "${CMAKE_COMMAND}" -E copy_if_different
          "${RUST_HOST_CARGO_BINARY}" "${RUST_HOST_BINARY}"
  DEPENDS "${RUST_HOST_CARGO_BINARY}"
  COMMENT "Staging the Rust host tracer"
  VERBATIM
)
add_custom_target(rust_workspace ALL
  DEPENDS "${PLATFORM_STATIC_LIB}" "${RUST_HOST_BINARY}")
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
add_dependencies(lynx-launcher rust_workspace)
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
  X11::X11
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
add_dependencies(rust_workspace runtime_resources)

if(BUILD_TESTING)
  add_executable(host_support_test
    tests/support_test.cc
  )
  target_link_libraries(host_support_test PRIVATE host_support)

  add_executable(x11_click_test_driver
    tests/x11_click_test_driver.cc
  )
  target_link_libraries(x11_click_test_driver PRIVATE
    host_support
    X11::X11
  )

  add_library(lynx_sys_c_header_smoke OBJECT
    tests/lynx_sys_c_header_smoke.c
  )
  target_include_directories(lynx_sys_c_header_smoke PRIVATE
    "${LYNX_SYS_DIR}/include"
  )

  add_executable(lynx_sys_abi_test
    tests/lynx_sys_abi_test.cc
    $<TARGET_OBJECTS:lynx_sys_c_header_smoke>
  )
  target_include_directories(lynx_sys_abi_test PRIVATE
    "${LYNX_SDK_DIR}/include"
    "${LYNX_SYS_DIR}/include"
  )
  target_compile_definitions(lynx_sys_abi_test PRIVATE
    USE_WEAK_SUFFIX_NAPI=1
  )
  target_link_libraries(lynx_sys_abi_test PRIVATE
    lynx_sys_shim
    lynx_sdk
  )

  add_test(NAME host_support_test COMMAND host_support_test)
  add_test(NAME lynx_sys_abi_test COMMAND lynx_sys_abi_test)
  add_test(NAME lynx_sys_rust_abi_test
    COMMAND "${CMAKE_COMMAND}" -E env
            "CARGO_TARGET_DIR=${CMAKE_BINARY_DIR}/cargo-abi-target"
            "LYNX_SDK_DIR=${LYNX_SDK_DIR}"
            "LYNX_LAUNCHER_LYNX_SHIM_ARCHIVE=$<TARGET_FILE:lynx_sys_shim>"
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=${CMAKE_CXX_COMPILER}"
            "LD_LIBRARY_PATH=${LYNX_SDK_DIR}/lib"
            cargo test --locked -p lynx-sys --test abi)
  set_tests_properties(lynx_sys_rust_abi_test PROPERTIES
    RUN_SERIAL TRUE
    WORKING_DIRECTORY "${LAUNCHER_ROOT}"
  )
  add_test(NAME launcher_resource_smoke
           COMMAND lynx-launcher --check-resources)
  add_test(NAME launcher_rejects_empty_icu
    COMMAND "${CMAKE_COMMAND}"
            "-DLAUNCHER=$<TARGET_FILE:lynx-launcher>"
            "-DTEST_ROOT=${CMAKE_CURRENT_BINARY_DIR}/empty-icu-test"
            -P "${CMAKE_CURRENT_LIST_DIR}/cmake/test_empty_icu.cmake")
  set(RUST_RESOURCE_TEST_WORKING_DIRECTORY
      "${CMAKE_CURRENT_BINARY_DIR}/rust-resource-smoke-working")
  file(MAKE_DIRECTORY "${RUST_RESOURCE_TEST_WORKING_DIRECTORY}")
  add_test(NAME rust_launcher_resource_smoke
           COMMAND "${RUST_HOST_BINARY}" --check-resources)
  set_tests_properties(rust_launcher_resource_smoke PROPERTIES
    WORKING_DIRECTORY "${RUST_RESOURCE_TEST_WORKING_DIRECTORY}")
  add_test(NAME rust_launcher_rejects_empty_icu
    COMMAND "${CMAKE_COMMAND}"
            "-DLAUNCHER=${RUST_HOST_BINARY}"
            "-DTEST_ROOT=${CMAKE_CURRENT_BINARY_DIR}/rust-empty-icu-test"
            -P "${CMAKE_CURRENT_LIST_DIR}/cmake/test_empty_icu.cmake")
  add_test(NAME rust_launcher_requires_display
    COMMAND "${CMAKE_COMMAND}"
            "-DLAUNCHER=${RUST_HOST_BINARY}"
            -P "${CMAKE_CURRENT_LIST_DIR}/cmake/test_rust_requires_display.cmake")
  add_test(NAME rust_launcher_rejects_external_lynx
    COMMAND "${CMAKE_COMMAND}"
            "-DLAUNCHER=${RUST_HOST_BINARY}"
            "-DSTAGED_LIBRARY=${RUNTIME_LIBLYNX}"
            "-DTEST_ROOT=${CMAKE_CURRENT_BINARY_DIR}/rust-external-lynx-test"
            -P "${CMAKE_CURRENT_LIST_DIR}/cmake/test_rust_rejects_external_lynx.cmake")
  add_test(NAME runtime_resource_refresh_older_source
    COMMAND "${CMAKE_COMMAND}"
            "-DREFRESH_SCRIPT=${REFRESH_RESOURCE_SCRIPT}"
            "-DTEST_ROOT=${CMAKE_CURRENT_BINARY_DIR}/resource-refresh-test"
            -P "${CMAKE_CURRENT_LIST_DIR}/cmake/test_resource_refresh.cmake")
endif()
