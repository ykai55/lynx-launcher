if(NOT DEFINED LAUNCHER)
  message(FATAL_ERROR "LAUNCHER is required")
endif()

execute_process(
  COMMAND "${CMAKE_COMMAND}" -E env "LC_ALL=C" readelf --wide --dynamic "${LAUNCHER}"
  RESULT_VARIABLE readelf_status
  OUTPUT_VARIABLE dynamic_section
  ERROR_VARIABLE readelf_error
)
if(NOT readelf_status EQUAL 0)
  message(FATAL_ERROR "readelf failed for ${LAUNCHER}:\n${readelf_error}")
endif()
if(NOT dynamic_section MATCHES "\\(RUNPATH\\).*\\[\\$ORIGIN\\]")
  message(FATAL_ERROR "${LAUNCHER} does not use an exact $ORIGIN runtime search path")
endif()

string(REGEX MATCHALL "Shared library: \\[[^]]+\\]" needed_entries "${dynamic_section}")
set(lynx_count 0)
foreach(entry IN LISTS needed_entries)
  string(REGEX REPLACE ".*\\[([^]]+)\\]" "\\1" library "${entry}")
  if(library STREQUAL "liblynx.so")
    math(EXPR lynx_count "${lynx_count} + 1")
  elseif(library MATCHES "^libglfw")
    message(FATAL_ERROR "${LAUNCHER} dynamically links forbidden GLFW library ${library}")
  elseif(NOT library MATCHES "^(libX11|libGLX|libOpenGL|libstdc\\+\\+|libgcc_s|libm|libc|ld-linux-x86-64)\\.so($|\\.)")
    message(FATAL_ERROR "${LAUNCHER} has unexpected direct dependency ${library}")
  endif()
endforeach()
if(NOT lynx_count EQUAL 1)
  message(FATAL_ERROR "${LAUNCHER} must have exactly one liblynx.so dependency, found ${lynx_count}")
endif()
