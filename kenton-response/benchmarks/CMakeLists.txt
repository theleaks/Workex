# kenton-response benchmarks — CMake config.
#
# Links against a locally-built V8 monolithic static library.
# V8_DIR must point at a v8 checkout built with:
#
#     gn gen out/x64.release --args='is_debug=false target_cpu="x64" \
#                                    v8_monolithic=true \
#                                    v8_use_external_startup_data=false \
#                                    use_custom_libcxx=false \
#                                    is_component_build=false'
#     ninja -C out/x64.release v8_monolith
#
# See build.sh for the full recipe.

cmake_minimum_required(VERSION 3.16)
project(kenton_response_bench CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_POSITION_INDEPENDENT_CODE ON)

if(NOT DEFINED V8_DIR)
  message(FATAL_ERROR
    "V8_DIR not set. Pass -DV8_DIR=/path/to/v8 when invoking cmake.")
endif()

set(V8_BUILD "${V8_DIR}/out/x64.release" CACHE PATH "V8 build output dir")
set(V8_MONOLITH "${V8_BUILD}/obj/libv8_monolith.a" CACHE FILEPATH
    "Path to libv8_monolith.a")

if(NOT EXISTS "${V8_MONOLITH}")
  message(FATAL_ERROR
    "libv8_monolith.a not found at ${V8_MONOLITH}. "
    "Build V8 first (see build.sh).")
endif()

add_library(v8_monolith STATIC IMPORTED)
set_target_properties(v8_monolith PROPERTIES
  IMPORTED_LOCATION "${V8_MONOLITH}"
  INTERFACE_INCLUDE_DIRECTORIES "${V8_DIR}/include"
)

add_definitions(-DV8_COMPRESS_POINTERS -DV8_ENABLE_SANDBOX)

# V8 is built with -fno-rtti by default. Our code must match, otherwise
# the linker fails on `typeinfo for v8::ValueSerializer::Delegate` when we
# derive from it. Also drop exceptions (V8 doesn't use them).
if(NOT MSVC)
  add_compile_options(-fno-rtti -fno-exceptions)
endif()

add_library(bench_common STATIC common.cc common.h)
target_link_libraries(bench_common PUBLIC v8_monolith)

# V8's ExternalEntityTable<T> where sizeof(T) > 16 emits calls to
# __atomic_compare_exchange (the generic N-byte form, in libatomic).
# Default linker --as-needed would drop libatomic because by the time
# the linker sees -latomic, nothing has yet referenced those symbols.
# --no-as-needed forces the lib to stay in the DT_NEEDED list so v8's
# symbols can find it.
set(V8_LINK bench_common "-Wl,--no-as-needed" atomic "-Wl,--as-needed"
            ${CMAKE_DL_LIBS} pthread)

add_executable(p1_serialization_cost p1_serialization_cost.cc)
target_link_libraries(p1_serialization_cost PRIVATE ${V8_LINK})

add_executable(p2_resume_cost p2_resume_cost.cc)
target_link_libraries(p2_resume_cost PRIVATE ${V8_LINK})
