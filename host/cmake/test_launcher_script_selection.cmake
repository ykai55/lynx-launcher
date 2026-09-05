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
   NOT run_error MATCHES "LYNX_LAUNCHER_WINDOW_BACKEND must be x11 or wayland")
  message(FATAL_ERROR
    "run.sh accepted an invalid backend or returned the wrong error:\n${run_output}${run_error}")
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

foreach(backend IN ITEMS x11 wayland)
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
