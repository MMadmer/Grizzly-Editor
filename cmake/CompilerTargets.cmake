# The level compilers and the editor libraries they link — ported from vcxproj specs verified
# against the MSBuild tlog command lines. Release|x64 parity.

set(SRC "${XRAY_SOURCE}")
set(ED  "${SRC}/Editors")

#-- FreeMagic (static) ---------------------------------------------------------
xray_glob(FREEMAGIC_SRC "${ED}/FreeMagic")
add_library(FreeMagic STATIC ${FREEMAGIC_SRC})
xray_common(FreeMagic NO_MBCS)
target_compile_definitions(FreeMagic PRIVATE FREEMAGIC_EXPORTS _WINDOWS _USRDLL)

#-- XrETools -------------------------------------------------------------------
xray_glob(XRETOOLS_SRC "${ED}/XrETools")
add_library(XrETools SHARED ${XRETOOLS_SRC})
xray_common(XrETools NO_MBCS PCH "${ED}/XrETools/stdafx.h")  # CharacterSet NotSet
xray_editor_cxx20(XrETools)
target_compile_definitions(XrETools PRIVATE XRETOOLS_EXPORTS _WINDOWS _USRDLL)
target_link_libraries(XrETools PRIVATE Ogg Vorbis XrCDB XrCore XrQSlim)

#-- XrDXT ----------------------------------------------------------------------
xray_glob(XRDXT_SRC "${ED}/XrDXT")
add_library(XrDXT SHARED ${XRDXT_SRC})
xray_common(XrDXT PCH "${ED}/XrDXT/stdafx.h")
xray_editor_cxx20(XrDXT)
target_compile_definitions(XrDXT PRIVATE XRDXT_EXPORTS _WINDOWS _USRDLL)
target_link_libraries(XrDXT PRIVATE RedImageTool XrCore)

#-- XrLCLight ------------------------------------------------------------------
xray_glob(XRLCLIGHT_SRC "${ED}/XrLCLight")
add_library(XrLCLight SHARED
    ${XRLCLIGHT_SRC}
    "${SRC}/XrEngine/xrLoadSurface.cpp"
    "${ED}/Public/hxGridInterface.cpp")
xray_common(XrLCLight)  # no PCH by design
target_compile_definitions(XrLCLight PRIVATE XRLC_LIGHT_EXPORTS _WINDOWS)
target_link_libraries(XrLCLight PRIVATE RedImageTool zlib XrCDB XrCore FreeMagic XrDXT)

#-- XrLC -----------------------------------------------------------------------
xray_glob(XRLC_SRC "${ED}/XrLC")
xray_glob(XRLC_NVM "${ED}/XrLC/NvMender2002")
xray_glob(XRLC_NVL "${ED}/XrLC/NV_Library")
add_executable(XrLC WIN32
    ${XRLC_SRC} ${XRLC_NVM} ${XRLC_NVL}
    "${ED}/Public/NVMeshMenderTwo.cpp"
    "${ED}/XrLC/resource.rc")
xray_common(XrLC)  # vcxproj creates a PCH nobody consumes — port without one
target_compile_definitions(XrLC PRIVATE _WINDOWS)
target_include_directories(XrLC PRIVATE "${ED}/XrLC")
target_link_libraries(XrLC PRIVATE
    RedImageTool zlib XrCDB XrCore FreeMagic XrDXT XrETools XrLCLight XrQSlim
    d3dx9 Winmm Comctl32)

#-- xrDO_Light -----------------------------------------------------------------
xray_glob(XRDO_SRC "${ED}/xrDO_Light")
add_executable(xrDO_Light WIN32
    ${XRDO_SRC}
    "${ED}/xrDO_Light/resource.rc")
xray_common(xrDO_Light PCH "${ED}/xrDO_Light/StdAfx.h")
target_compile_definitions(xrDO_Light PRIVATE _WINDOWS)
target_link_libraries(xrDO_Light PRIVATE XrCore XrLCLight d3dx9 Winmm Comctl32)
