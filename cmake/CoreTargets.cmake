# The engine libraries the level compilers link — ported 1:1 from the vcxproj specs (Release|x64).

set(SRC "${XRAY_SOURCE}")

#-- XrAPI ----------------------------------------------------------------------
xray_glob(XRAPI_SRC "${SRC}/XrAPI")
add_library(XrAPI SHARED ${XRAPI_SRC})
xray_common(XrAPI PCH "${SRC}/XrAPI/stdafx.h")
target_compile_definitions(XrAPI PRIVATE XRAPI_EXPORTS _WINDOWS _USRDLL)

#-- XrCore ---------------------------------------------------------------------
xray_glob(XRCORE_SRC "${SRC}/XrCore")
set(XRCORE_C
    "${SRC}/XrCore/ptmalloc3/malloc.c"
    "${SRC}/XrCore/ptmalloc3/sysdeps/win32/win32.c"
)
add_library(XrCore SHARED ${XRCORE_SRC} ${XRCORE_C})
xray_common(XrCore PCH "${SRC}/XrCore/stdafx.h")
target_compile_definitions(XrCore PRIVATE XRCORE_EXPORTS _WINDOWS _USRDLL)
target_link_libraries(XrCore
    PRIVATE XrAPI dxerr comdlg32 LZO
)
target_include_directories(XrCore PUBLIC "${LZO_ROOT}/include")
xray_no_pch(${XRCORE_C} "${SRC}/XrCore/Model.cpp")

#-- XrCDB ----------------------------------------------------------------------
xray_glob(XRCDB_SRC "${SRC}/XrCDB")
add_library(XrCDB SHARED ${XRCDB_SRC})
xray_common(XrCDB PCH "${SRC}/XrCDB/stdafx.h")
target_compile_definitions(XrCDB PRIVATE XRCDB_EXPORTS _WINDOWS _USRDLL)
target_link_libraries(XrCDB PRIVATE XrAPI XrCore winmm)

#-- XrQSlim (static) -----------------------------------------------------------
xray_glob(XRQSLIM_SRC "${SRC}/Editors/XrQSlim")
add_library(XrQSlim STATIC ${XRQSLIM_SRC})
xray_common(XrQSlim NO_MBCS)
target_compile_definitions(XrQSlim PRIVATE _LIB)
