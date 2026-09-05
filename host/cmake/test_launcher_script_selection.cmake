if(NOT DEFINED SCRIPTS_DIR)
  message(FATAL_ERROR "SCRIPTS_DIR is required")
endif()

execute_process(
  COMMAND "${CMAKE_COMMAND}" -E env
          "LYNX_LAUNCHER_WINDOW_BACKEND=invalid"
          bash "${SCRIPTS_DIR}/run.sh"
  RESULT_VARIABLE run_status
  OUTPUT_VARIABLE run_output
  ERROR_VARIABLE run_error
)
if(run_status EQUAL 0 OR
   NOT run_error MATCHES "LYNX_LAUNCHER_WINDOW_BACKEND must be auto, x11, or wayland")
  message(FATAL_ERROR
    "run.sh accepted an invalid backend or returned the wrong error:\n${run_output}${run_error}")
endif()

set(test_root "${CMAKE_CURRENT_BINARY_DIR}/launcher-script-selection-test")
file(REMOVE_RECURSE "${test_root}")
file(MAKE_DIRECTORY
  "${test_root}/bin"
  "${test_root}/scripts"
  "${test_root}/host/build/resources"
  "${test_root}/host/build-wayland/resources")
configure_file("${SCRIPTS_DIR}/run.sh" "${test_root}/scripts/run.sh" COPYONLY)
configure_file("${SCRIPTS_DIR}/../host/build.sh" "${test_root}/host/build.sh" COPYONLY)
file(WRITE "${test_root}/scripts/_common.sh"
  "host_build_dir=\"${test_root}/host/build\"\n"
  "rust_host_binary=\"${test_root}/host/build/lynx-launcher\"\n"
  "host_wayland_build_dir=\"${test_root}/host/build-wayland\"\n"
  "wayland_host_binary=\"${test_root}/host/build-wayland/lynx-launcher-wayland\"\n"
  "verified_sdk_dir=\"${test_root}/sdk\"\n"
  "die() { printf 'error: %s\\n' \"$*\" >&2; exit 1; }\n"
  "require_command() { :; }\n"
  "ensure_sdk() { :; }\n")
file(WRITE "${test_root}/bin/cmake"
  "#!/usr/bin/env bash\n"
  "if [[ -n \"$EXPECT_REMOVED\" && -e \"$EXPECT_REMOVED\" ]]; then\n"
  "  printf 'expected stale executable to be removed before cmake: %s\\n' \"$EXPECT_REMOVED\" >&2\n"
  "  exit 1\n"
  "fi\n"
  "if [[ -n \"$EXPECT_PRESERVED\" && ! -e \"$EXPECT_PRESERVED\" ]]; then\n"
  "  printf 'expected executable to be preserved before cmake: %s\\n' \"$EXPECT_PRESERVED\" >&2\n"
  "  exit 1\n"
  "fi\n")
file(WRITE "${test_root}/bin/ctest" "#!/usr/bin/env bash\nexit 0\n")
execute_process(
  COMMAND chmod +x "${test_root}/bin/cmake" "${test_root}/bin/ctest"
  RESULT_VARIABLE fixture_chmod_status)
if(NOT fixture_chmod_status EQUAL 0)
  message(FATAL_ERROR "could not make host build contract fixtures executable")
endif()

function(assert_host_build_invalidation test_name build_dir should_remove)
  file(WRITE "${test_root}/host/build-wayland/lynx-launcher-wayland" "stale")
  set(environment
    "${CMAKE_COMMAND}" -E env
    "PATH=${test_root}/bin:$ENV{PATH}")
  if(build_dir STREQUAL "DEFAULT")
    list(APPEND environment --unset=LYNX_LAUNCHER_HOST_BUILD_DIR)
  else()
    list(APPEND environment "LYNX_LAUNCHER_HOST_BUILD_DIR=${build_dir}")
  endif()
  if(should_remove)
    list(APPEND environment
      "EXPECT_REMOVED=${test_root}/host/build-wayland/lynx-launcher-wayland"
      "EXPECT_PRESERVED=")
  else()
    list(APPEND environment
      "EXPECT_REMOVED="
      "EXPECT_PRESERVED=${test_root}/host/build-wayland/lynx-launcher-wayland")
  endif()
  execute_process(
    COMMAND ${environment} bash "${test_root}/host/build.sh"
    RESULT_VARIABLE host_build_status
    OUTPUT_VARIABLE host_build_output
    ERROR_VARIABLE host_build_error)
  if(NOT host_build_status EQUAL 0)
    message(FATAL_ERROR
      "host/build.sh failed ${test_name}:\n${host_build_output}${host_build_error}")
  endif()
  if(should_remove AND
     EXISTS "${test_root}/host/build-wayland/lynx-launcher-wayland")
    message(FATAL_ERROR "host/build.sh did not invalidate stale Wayland executable")
  elseif(NOT should_remove AND
         NOT EXISTS "${test_root}/host/build-wayland/lynx-launcher-wayland")
    message(FATAL_ERROR "host/build.sh deleted the executable during ${test_name}")
  endif()
endfunction()

assert_host_build_invalidation("canonical X11 incremental build" DEFAULT TRUE)
assert_host_build_invalidation(
  "custom incremental build" "${test_root}/host/custom-build" FALSE)
assert_host_build_invalidation(
  "native Wayland incremental build" "${test_root}/host/build-wayland" FALSE)

foreach(build_dir IN ITEMS build build-wayland)
  file(WRITE "${test_root}/host/${build_dir}/liblynx.so" "test")
  file(WRITE "${test_root}/host/${build_dir}/lynx_core.js" "test")
  file(WRITE "${test_root}/host/${build_dir}/resources/icudtl.dat" "test")
  file(WRITE "${test_root}/host/${build_dir}/resources/lynx_core.js" "test")
  file(WRITE "${test_root}/host/${build_dir}/resources/main.lynx.bundle" "test")
endforeach()
file(WRITE "${test_root}/host/build/lynx-launcher"
  "#!/usr/bin/env bash\nprintf 'x11:%s\\n' \"$*\"\n")
file(WRITE "${test_root}/host/build-wayland/lynx-launcher-wayland"
  "#!/usr/bin/env bash\nprintf 'wayland:%s\\n' \"$*\"\n")
execute_process(
  COMMAND chmod +x
          "${test_root}/host/build/lynx-launcher"
          "${test_root}/host/build-wayland/lynx-launcher-wayland"
  RESULT_VARIABLE chmod_status)
if(NOT chmod_status EQUAL 0)
  message(FATAL_ERROR "could not make launcher contract fixtures executable")
endif()

function(assert_run_selection test_name expected_backend backend display)
  set(environment "${CMAKE_COMMAND}" -E env)
  if(backend STREQUAL "UNSET")
    list(APPEND environment --unset=LYNX_LAUNCHER_WINDOW_BACKEND)
  else()
    list(APPEND environment "LYNX_LAUNCHER_WINDOW_BACKEND=${backend}")
  endif()
  if(display STREQUAL "UNSET")
    list(APPEND environment --unset=WAYLAND_DISPLAY)
  else()
    list(APPEND environment "WAYLAND_DISPLAY=${display}")
  endif()
  execute_process(
    COMMAND ${environment} bash "${test_root}/scripts/run.sh" contract-marker
    RESULT_VARIABLE selection_status
    OUTPUT_VARIABLE selection_output
    ERROR_VARIABLE selection_error)
  if(NOT selection_status EQUAL 0 OR
     NOT selection_output STREQUAL
         "${expected_backend}:--window-backend ${expected_backend} contract-marker\n")
    message(FATAL_ERROR
      "run.sh failed ${test_name}:\n${selection_output}${selection_error}")
  endif()
endfunction()

assert_run_selection("default auto Wayland selection" wayland UNSET wayland-0)
assert_run_selection("default auto X11 selection without WAYLAND_DISPLAY" x11 UNSET UNSET)
assert_run_selection("default auto X11 selection with empty WAYLAND_DISPLAY" x11 UNSET "")
assert_run_selection("explicit auto selection" wayland auto wayland-0)
assert_run_selection("strict explicit X11 selection" x11 x11 wayland-0)
assert_run_selection("strict explicit Wayland selection" wayland wayland UNSET)

execute_process(
  COMMAND chmod -x "${test_root}/host/build-wayland/lynx-launcher-wayland"
  RESULT_VARIABLE chmod_status)
if(NOT chmod_status EQUAL 0)
  message(FATAL_ERROR "could not make the Wayland launcher fixture non-executable")
endif()
assert_run_selection("auto X11 selection without executable Wayland host" x11 auto wayland-0)
file(REMOVE "${test_root}/host/build-wayland/lynx-launcher-wayland")

execute_process(
  COMMAND "${CMAKE_COMMAND}" -E env
          "LYNX_LAUNCHER_WINDOW_BACKEND=wayland"
          bash "${test_root}/scripts/run.sh"
  RESULT_VARIABLE missing_wayland_status
  OUTPUT_VARIABLE missing_wayland_output
  ERROR_VARIABLE missing_wayland_error)
if(missing_wayland_status EQUAL 0 OR
   NOT missing_wayland_error MATCHES
       "built wayland runtime is missing .*lynx-launcher-wayland; run LYNX_LAUNCHER_BUILD_WAYLAND=1 ./scripts/build.sh first")
  message(FATAL_ERROR
    "run.sh did not keep explicit Wayland selection strict or return its build hint:\n${missing_wayland_output}${missing_wayland_error}")
endif()

execute_process(
  COMMAND "${CMAKE_COMMAND}" -E env
          "LYNX_LAUNCHER_BUILD_WAYLAND=true"
          bash "${SCRIPTS_DIR}/build.sh"
  RESULT_VARIABLE build_status
  OUTPUT_VARIABLE build_output
  ERROR_VARIABLE build_error
)
if(build_status EQUAL 0 OR
   NOT build_error MATCHES "LYNX_LAUNCHER_BUILD_WAYLAND must be 0 or 1")
  message(FATAL_ERROR
    "build.sh accepted an invalid Wayland boolean or returned the wrong error:\n${build_output}${build_error}")
endif()

execute_process(
  COMMAND "${CMAKE_COMMAND}" -E env
          "LYNX_LAUNCHER_WINDOW_BACKEND=invalid"
          bash "${SCRIPTS_DIR}/teardown-stress.sh"
  RESULT_VARIABLE teardown_backend_status
  OUTPUT_VARIABLE teardown_backend_output
  ERROR_VARIABLE teardown_backend_error
)
if(teardown_backend_status EQUAL 0 OR
   NOT teardown_backend_error MATCHES
       "LYNX_LAUNCHER_WINDOW_BACKEND must be x11 or wayland")
  message(FATAL_ERROR
    "teardown-stress.sh accepted an invalid backend or returned the wrong error:\n${teardown_backend_output}${teardown_backend_error}")
endif()

execute_process(
  COMMAND "${CMAKE_COMMAND}" -E env --unset=WAYLAND_DISPLAY
          "LYNX_LAUNCHER_WINDOW_BACKEND=wayland"
          bash "${SCRIPTS_DIR}/teardown-stress.sh"
  RESULT_VARIABLE teardown_wayland_status
  OUTPUT_VARIABLE teardown_wayland_output
  ERROR_VARIABLE teardown_wayland_error
)
if(teardown_wayland_status EQUAL 0 OR
   NOT teardown_wayland_error MATCHES
       "native Wayland teardown stress test requires WAYLAND_DISPLAY")
  message(FATAL_ERROR
    "teardown-stress.sh did not require WAYLAND_DISPLAY in Wayland mode:\n${teardown_wayland_output}${teardown_wayland_error}")
endif()

file(READ "${SCRIPTS_DIR}/teardown-stress.sh" teardown_script)
foreach(required_fragment IN ITEMS
    [[window_backend="${LYNX_LAUNCHER_WINDOW_BACKEND-x11}"]]
    [[selected_host_binary="${wayland_host_binary}"]]
    [[backend_arguments=(--window-backend wayland)]]
    [[-u LYNX_LAUNCHER_BUNDLE]]
    [[-u LYNX_LAUNCHER_ICU]]
    [[-u LYNX_LAUNCHER_LYNX_CORE]]
    [[-u LYNX_LAUNCHER_WINDOW_BACKEND]])
  string(FIND "${teardown_script}" "${required_fragment}" fragment_position)
  if(fragment_position EQUAL -1)
    message(FATAL_ERROR
      "teardown-stress.sh is missing selector contract fragment: ${required_fragment}")
  endif()
endforeach()

foreach(backend IN ITEMS auto x11 wayland)
  foreach(arguments IN ITEMS "--window-backend;wayland" "--window-backend=x11")
    execute_process(
      COMMAND "${CMAKE_COMMAND}" -E env
              "LYNX_LAUNCHER_WINDOW_BACKEND=${backend}"
              bash "${SCRIPTS_DIR}/run.sh" ${arguments}
      RESULT_VARIABLE duplicate_status
      OUTPUT_VARIABLE duplicate_output
      ERROR_VARIABLE duplicate_error
    )
    if(duplicate_status EQUAL 0 OR
       NOT duplicate_error MATCHES
           "select the window backend only with LYNX_LAUNCHER_WINDOW_BACKEND")
      message(FATAL_ERROR
        "run.sh accepted a second backend selection for ${backend}:\n${duplicate_output}${duplicate_error}")
    endif()
  endforeach()
endforeach()

file(REMOVE_RECURSE "${test_root}")
