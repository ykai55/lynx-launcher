if(NOT DEFINED LAUNCHER OR NOT DEFINED NATIVE_WAYLAND_COMPILED)
  message(FATAL_ERROR "LAUNCHER and NATIVE_WAYLAND_COMPILED are required")
endif()

execute_process(
  COMMAND "${CMAKE_COMMAND}" -E env --unset=WAYLAND_DISPLAY
          "${LAUNCHER}" --window-backend wayland --exit-after-first-frame
  RESULT_VARIABLE launcher_status
  OUTPUT_VARIABLE launcher_output
  ERROR_VARIABLE launcher_error
)
if(launcher_status EQUAL 0)
  message(FATAL_ERROR
    "Rust native Wayland backend ran without WAYLAND_DISPLAY:\n${launcher_output}${launcher_error}")
endif()
if(NATIVE_WAYLAND_COMPILED)
  set(expected_error "WAYLAND_DISPLAY is not set")
else()
  set(expected_error "native Wayland support is not compiled in")
endif()
if(NOT launcher_error MATCHES "${expected_error}")
  message(FATAL_ERROR
    "Rust native Wayland backend failed for the wrong reason:\n${launcher_output}${launcher_error}")
endif()
