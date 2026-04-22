# kenton-response V8-COW support — separate CMakeLists so the standalone
# prototype and the two benchmarks can build independently of the P1/P2
# benchmarks in ../../benchmarks.

cmake_minimum_required(VERSION 3.16)
project(kenton_response_v8_cow CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# The standalone prototype doesn't depend on V8 at all.
add_executable(cow_builtins_prototype cow_builtins_prototype.cc)

# memory_benchmark and heap_classifier link V8.
if(DEFINED V8_DIR)
  set(V8_BUILD "${V8_DIR}/out/x64.release" CACHE PATH "V8 build dir")
  set(V8_MONOLITH "${V8_BUILD}/obj/libv8_monolith.a" CACHE FILEPATH "")
  if(EXISTS "${V8_MONOLITH}")
    add_library(v8_monolith STATIC IMPORTED)
    set_target_properties(v8_monolith PROPERTIES
      IMPORTED_LOCATION "${V8_MONOLITH}"
      INTERFACE_INCLUDE_DIRECTORIES "${V8_DIR}/include"
    )
    add_definitions(-DV8_COMPRESS_POINTERS -DV8_ENABLE_SANDBOX)

    # Match V8's -fno-rtti -fno-exceptions; force libatomic retention so
    # ld keeps it in DT_NEEDED when v8_monolith symbols pull it in.
    if(NOT MSVC)
      add_compile_options(-fno-rtti -fno-exceptions)
    endif()
    set(V8_LINK v8_monolith "-Wl,--no-as-needed" atomic "-Wl,--as-needed"
                ${CMAKE_DL_LIBS} pthread)

    add_executable(memory_benchmark memory_benchmark.cc)
    target_link_libraries(memory_benchmark PRIVATE ${V8_LINK})

    add_executable(heap_classifier heap_classifier.cc)
    target_link_libraries(heap_classifier PRIVATE ${V8_LINK})
  else()
    message(WARNING
      "V8 monolith not found at ${V8_MONOLITH} — V8-dependent targets "
      "will not be built. Build V8 via "
      "../../benchmarks/fetch_and_build_v8.sh first.")
  endif()
else()
  message(STATUS
    "V8_DIR not set — only building cow_builtins_prototype (no V8 dep).")
endif()
