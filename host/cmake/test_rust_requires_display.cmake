if(NOT DEFINED LAUNCHER)
  message(FATAL_ERROR "LAUNCHER is required")
endif()

execute_process(
  COMMAND "${CMAKE_COMMAND}" -E env --unset=DISPLAY
          "${LAUNCHER}" --exit-after-first-frame
  RESULT_VARIABLE launcher_status
  OUTPUT_VARIABLE launcher_output
  ERROR_VARIABLE launcher_error
)
if(launcher_status EQUAL 0)
  message(FATAL_ERROR
    "Rust window shell ran without DISPLAY:\n${launcher_output}${launcher_error}")
endif()
if(NOT launcher_error MATCHES "DISPLAY is not set")
  message(FATAL_ERROR
    "Rust window shell failed for the wrong reason:\n${launcher_output}${launcher_error}")
endif()
