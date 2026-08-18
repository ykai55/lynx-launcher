if(NOT DEFINED LAUNCHER OR NOT DEFINED TEST_ROOT)
  message(FATAL_ERROR "LAUNCHER and TEST_ROOT are required")
endif()

file(REMOVE_RECURSE "${TEST_ROOT}")
file(MAKE_DIRECTORY "${TEST_ROOT}/working")
set(empty_icu "${TEST_ROOT}/empty-icudtl.dat")
file(WRITE "${empty_icu}" "")

execute_process(
  COMMAND "${LAUNCHER}" --check-resources --icu "${empty_icu}"
  WORKING_DIRECTORY "${TEST_ROOT}/working"
  RESULT_VARIABLE check_status
  OUTPUT_VARIABLE check_output
  ERROR_VARIABLE check_error
)
if(check_status EQUAL 0)
  message(FATAL_ERROR
    "Resource check accepted an empty ICU file:\n${check_output}${check_error}")
endif()
if(NOT check_error MATCHES "resource is empty")
  message(FATAL_ERROR
    "Resource check failed for the wrong reason:\n${check_output}${check_error}")
endif()

file(REMOVE_RECURSE "${TEST_ROOT}")
