# The level compilers XrLC, XrLCLight and xrDO_Light (CMake + Ninja), the last C++ of the
# repository until they become Rust (docs/RUST_MIGRATION.md, phase 8). Build.ps1 builds the Rust
# editor and then runs this.
#
# Ported from the vcxproj files of cpp-editor-last:Source/Engine.sln (Release|x64), including
# their quirks (DEBUG defined in Release, /W0, /arch:AVX). The specs live in cmake/*.cmake.

cmake_minimum_required(VERSION 3.25)
project(GrizzlyCompilers LANGUAGES C CXX RC)

if (NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
endif()
set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreadedDLL")   # /MD everywhere, like the sln

# parity: the x64 vcxproj configs deliberately do NOT define WIN32 — strip the
# default CMake/MSVC platform define before it poisons legacy #ifdef WIN32 code
foreach(_lang C CXX)
    string(REPLACE "/DWIN32" "" CMAKE_${_lang}_FLAGS "${CMAKE_${_lang}_FLAGS}")
    string(REPLACE "-DWIN32" "" CMAKE_${_lang}_FLAGS "${CMAKE_${_lang}_FLAGS}")
endforeach()

include(cmake/XRayCommon.cmake)

# Keep the notices beside the binaries, including targeted builds.
file(MAKE_DIRECTORY "${XRAY_BIN}")
configure_file(
    "${XRAY_SOURCE}/External/THIRD_PARTY_NOTICES.txt"
    "${XRAY_BIN}/THIRD_PARTY_NOTICES.txt"
    COPYONLY
)

include(cmake/ExternalTargets.cmake)
include(cmake/LzoTarget.cmake)
include(cmake/CoreTargets.cmake)
include(cmake/CompilerTargets.cmake)
