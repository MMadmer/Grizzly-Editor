# Shared build rules ported from the property sheets cpp-editor-last:Source/Default.props and
# cpp-editor-last:Source/Default.Release.props.
# (Source/ at the tag cpp-editor-last).
# Parity notes:
#  - Release compiles with BOTH NDEBUG (per-project) and DEBUG (the Release property sheet)
#  - warnings are fully off (/W0), RTTI on, C++17, /arch:AVX, /MD, LTCG
#  - WIN32 is intentionally NOT defined on x64
#  - outputs: dlls/exes -> Bin/x64/<cfg>, import+static libs -> Lib/x64/<cfg>
#    (XRAY_BIN_DIR / XRAY_LIB_DIR name others)

set(XRAY_ROOT    "${CMAKE_SOURCE_DIR}")
set(XRAY_SOURCE  "${XRAY_ROOT}/Source")
set(XRAY_CFG_DIR "x64/${CMAKE_BUILD_TYPE}")
set(XRAY_BIN_DIR "" CACHE PATH "Where the binaries go (empty: Bin/x64/<config>)")
set(XRAY_LIB_DIR "" CACHE PATH "Where the import and static libraries go (empty: Lib/x64/<config>)")
set(XRAY_BIN     "${XRAY_ROOT}/Bin/${XRAY_CFG_DIR}")
set(XRAY_LIB     "${XRAY_ROOT}/Lib/${XRAY_CFG_DIR}")
if (XRAY_BIN_DIR)
    set(XRAY_BIN "${XRAY_BIN_DIR}")
endif()
if (XRAY_LIB_DIR)
    set(XRAY_LIB "${XRAY_LIB_DIR}")
endif()

option(XRAY_LTO "Whole-program optimization (/GL + /LTCG) like the old vcxproj" ON)

# every target routes through this to get the property sheet's behaviour
function(xray_common target)
    cmake_parse_arguments(ARG "NO_MBCS" "PCH" "" ${ARGN})

    set_target_properties(${target} PROPERTIES
        RUNTIME_OUTPUT_DIRECTORY "${XRAY_BIN}"
        LIBRARY_OUTPUT_DIRECTORY "${XRAY_BIN}"
        ARCHIVE_OUTPUT_DIRECTORY "${XRAY_LIB}"
        PDB_OUTPUT_DIRECTORY     "${XRAY_BIN}"
    )

    target_compile_definitions(${target} PRIVATE
        _SILENCE_STDEXT_HASH_DEPRECATION_WARNINGS
        _WINDLL
        _SECURE_SCL=0
        _ITERATOR_DEBUG_LEVEL=0
        # the old v142 STL still shipped auto_ptr/binary_function; the modern
        # one removes them unless this compatibility switch is set
        _HAS_AUTO_PTR_ETC=1
        $<$<CONFIG:Release>:DEBUG>      # sic — the Release property sheet does exactly this
        $<$<CONFIG:Release>:NDEBUG>
    )
    if (NOT ARG_NO_MBCS)
        target_compile_definitions(${target} PRIVATE _MBCS)
    endif()

    target_include_directories(${target} PRIVATE
        "${XRAY_ROOT}/SDK/Include"
        "${XRAY_SOURCE}/XrRender/Public"
        "${XRAY_SOURCE}/External/Public"
        "${XRAY_SOURCE}/External/RedImage"
    )

    target_compile_options(${target} PRIVATE
        # /W0 silences the legacy tree, but C4129 stays an ERROR: an
        # unrecognized escape is the compiler quietly DROPPING a backslash, so
        # "%s\%s" becomes "%s%s" and a path is built without its separator.
        # Cost a silent "Create > NPC Scenario does nothing" once already.
        /W0 /we4129 /GR /Oi /Gy /arch:AVX
        # restores the C++03 functional helpers the modern STL dropped
        $<$<COMPILE_LANGUAGE:CXX>:/FI${XRAY_SOURCE}/xr_msvc_stl_compat.h>
    )
    # /std:c++17 via the standard property so cmake does not fight us. The
    # C++03 helpers the modern STL dropped come back through the force-included
    # xr_msvc_stl_compat.h above, which is standard-agnostic by design.
    set_target_properties(${target} PROPERTIES CXX_STANDARD 17 CXX_STANDARD_REQUIRED ON)

    if (XRAY_LTO AND CMAKE_BUILD_TYPE STREQUAL "Release")
        target_compile_options(${target} PRIVATE /GL)
        target_link_options(${target} PRIVATE /LTCG)
    endif()

    target_link_directories(${target} PRIVATE
        "${XRAY_LIB}"
        "${XRAY_ROOT}/SDK/Lib/x64"
    )
    target_link_options(${target} PRIVATE /LARGEADDRESSAWARE /SUBSYSTEM:WINDOWS)

    if (ARG_PCH)
        target_precompile_headers(${target} PRIVATE "${ARG_PCH}")
        # no vcxproj ever fed a .c file through a PCH — and letting CMake emit
        # a C-language PCH would compile the C++ stdafx.h as C
        get_target_property(_srcs ${target} SOURCES)
        set(_c_srcs "${_srcs}")
        list(FILTER _c_srcs INCLUDE REGEX "\\.c$")
        if (_c_srcs)
            set_source_files_properties(${_c_srcs} PROPERTIES SKIP_PRECOMPILE_HEADERS ON)
        endif()
    endif()
endfunction()

# Raises first-party editor modules without changing the legacy runtime or vendor targets.
function(xray_editor_cxx20 target)
    set_target_properties(${target} PROPERTIES
        CXX_STANDARD 20
        CXX_STANDARD_REQUIRED ON
    )
    target_compile_definitions(${target} PRIVATE
        _HAS_DEPRECATED_ALLOCATOR_VOID=1
        _HAS_DEPRECATED_ALLOCATOR_MEMBERS=1
    )
    # /std:c++20 implies /permissive- in current MSVC, so restore the established parser mode explicitly.
    target_compile_options(${target} PRIVATE
        $<$<COMPILE_LANGUAGE:CXX>:/permissive>
    )
endfunction()

# glob helper: "all .cpp under DIR except EXCLUDE names" (case-insensitive)
function(xray_glob out_var dir)
    cmake_parse_arguments(ARG "" "" "EXCLUDE" ${ARGN})
    file(GLOB _files CONFIGURE_DEPENDS "${dir}/*.cpp")
    if (ARG_EXCLUDE)
        string(TOLOWER "${ARG_EXCLUDE}" _excl_lower)
        set(_kept "")
        foreach(_f IN LISTS _files)
            get_filename_component(_name "${_f}" NAME)
            string(TOLOWER "${_name}" _name_lower)
            if (NOT _name_lower IN_LIST _excl_lower)
                list(APPEND _kept "${_f}")
            endif()
        endforeach()
        set(_files "${_kept}")
    endif()
    set(${out_var} "${_files}" PARENT_SCOPE)
endfunction()

# mark files that must build without the target's PCH
function(xray_no_pch)
    set_source_files_properties(${ARGN} PROPERTIES SKIP_PRECOMPILE_HEADERS ON)
endfunction()
