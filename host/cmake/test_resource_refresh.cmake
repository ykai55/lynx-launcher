if(NOT DEFINED REFRESH_SCRIPT OR NOT DEFINED TEST_ROOT)
  message(FATAL_ERROR "REFRESH_SCRIPT and TEST_ROOT are required")
endif()

file(REMOVE_RECURSE "${TEST_ROOT}")
file(MAKE_DIRECTORY "${TEST_ROOT}")
set(source "${TEST_ROOT}/verified-source.dat")
set(destination "${TEST_ROOT}/runtime-destination.dat")

file(WRITE "${source}" "verified older source\n")
execute_process(COMMAND "${CMAKE_COMMAND}" -E sleep 2)
file(WRITE "${destination}" "stale newer destination\n")
file(TIMESTAMP "${source}" source_timestamp "%Y%m%d%H%M%S" UTC)
file(TIMESTAMP "${destination}" destination_timestamp "%Y%m%d%H%M%S" UTC)
if(NOT source_timestamp STRLESS destination_timestamp)
  message(FATAL_ERROR "Test setup did not make the source older than destination")
endif()

execute_process(
  COMMAND "${CMAKE_COMMAND}"
          "-DSOURCE=${source}"
          "-DDESTINATION=${destination}"
          -P "${REFRESH_SCRIPT}"
  RESULT_VARIABLE refresh_status
)
if(NOT refresh_status EQUAL 0)
  message(FATAL_ERROR "Resource refresh command failed")
endif()
file(READ "${destination}" refreshed_content)
if(NOT refreshed_content STREQUAL "verified older source\n")
  message(FATAL_ERROR "Newer destination was not refreshed from the older source")
endif()

file(REMOVE_RECURSE "${TEST_ROOT}")
