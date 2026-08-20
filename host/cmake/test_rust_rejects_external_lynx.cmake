if(NOT DEFINED LAUNCHER OR NOT DEFINED STAGED_LIBRARY OR NOT DEFINED TEST_ROOT)
  message(FATAL_ERROR "LAUNCHER, STAGED_LIBRARY, and TEST_ROOT are required")
endif()

file(REMOVE_RECURSE "${TEST_ROOT}")
file(MAKE_DIRECTORY "${TEST_ROOT}")
configure_file("${STAGED_LIBRARY}" "${TEST_ROOT}/liblynx.so" COPYONLY)

execute_process(
  COMMAND "${CMAKE_COMMAND}" -E env --unset=DISPLAY
          "LD_LIBRARY_PATH=${TEST_ROOT}"
          "${LAUNCHER}" --exit-after-first-frame
  RESULT_VARIABLE launcher_status
  OUTPUT_VARIABLE launcher_output
  ERROR_VARIABLE launcher_error
)
if(launcher_status EQUAL 0)
  message(FATAL_ERROR
    "Rust window shell accepted an external liblynx.so:\n${launcher_output}${launcher_error}")
endif()
if(NOT launcher_error MATCHES "loaded liblynx.so from")
  message(FATAL_ERROR
    "Rust window shell failed for the wrong reason:\n${launcher_output}${launcher_error}")
endif()

file(REMOVE_RECURSE "${TEST_ROOT}")
