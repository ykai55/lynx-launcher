if(NOT DEFINED SOURCE OR NOT DEFINED DESTINATION)
  message(FATAL_ERROR "SOURCE and DESTINATION are required")
endif()
if(NOT EXISTS "${SOURCE}")
  message(FATAL_ERROR "Runtime resource source is missing: ${SOURCE}")
endif()

get_filename_component(destination_directory "${DESTINATION}" DIRECTORY)
file(MAKE_DIRECTORY "${destination_directory}")
execute_process(
  COMMAND "${CMAKE_COMMAND}" -E copy_if_different "${SOURCE}" "${DESTINATION}"
  RESULT_VARIABLE copy_status
)
if(NOT copy_status EQUAL 0)
  message(FATAL_ERROR
    "Could not refresh runtime resource ${SOURCE} -> ${DESTINATION}")
endif()
