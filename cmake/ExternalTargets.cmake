# The third-party libraries the level compilers link, under Source/External — ported from vcxproj
# specs. All take the rules of cpp-editor-last:Source/Default.props (xray_common) except the RedImage
# subtree, which has its own minimal property sheet, and zlib.

set(SRC "${XRAY_SOURCE}")
set(EXT "${SRC}/External")

#-- Xiph codecs (static) -------------------------------------------------------
add_library(Ogg STATIC "${EXT}/Ogg/bitwise.c" "${EXT}/Ogg/framing.c")
xray_common(Ogg)
target_compile_definitions(Ogg PRIVATE _LIB)

set(VORBIS_SRC
    "${EXT}/Vorbis/analysis.c" "${EXT}/Vorbis/bitrate.c" "${EXT}/Vorbis/block.c"
    "${EXT}/Vorbis/codebook.c" "${EXT}/Vorbis/envelope.c" "${EXT}/Vorbis/floor0.c"
    "${EXT}/Vorbis/floor1.c" "${EXT}/Vorbis/info.c" "${EXT}/Vorbis/lookup.c"
    "${EXT}/Vorbis/lpc.c" "${EXT}/Vorbis/lsp.c" "${EXT}/Vorbis/mapping0.c"
    "${EXT}/Vorbis/mdct.c" "${EXT}/Vorbis/psy.c" "${EXT}/Vorbis/registry.c"
    "${EXT}/Vorbis/res0.c" "${EXT}/Vorbis/sharedbook.c" "${EXT}/Vorbis/smallft.c"
    "${EXT}/Vorbis/synthesis.c" "${EXT}/Vorbis/vorbisenc.c"
    "${EXT}/Vorbis/vorbisfile.c" "${EXT}/Vorbis/window.c")
add_library(Vorbis STATIC ${VORBIS_SRC})
xray_common(Vorbis)
target_compile_definitions(Vorbis PRIVATE _LIB)
target_include_directories(Vorbis PRIVATE "${EXT}/Vorbis")

# zlib: the one project without /arch:AVX — configured by hand for parity
set(ZLIB_SRC
    "${EXT}/zlib/adler32.c" "${EXT}/zlib/compress.c" "${EXT}/zlib/crc32.c"
    "${EXT}/zlib/deflate.c" "${EXT}/zlib/gzclose.c" "${EXT}/zlib/gzlib.c"
    "${EXT}/zlib/gzread.c" "${EXT}/zlib/gzwrite.c" "${EXT}/zlib/infback.c"
    "${EXT}/zlib/inffast.c" "${EXT}/zlib/inflate.c" "${EXT}/zlib/inftrees.c"
    "${EXT}/zlib/ioapi.c" "${EXT}/zlib/iowin32.c" "${EXT}/zlib/mztools.c"
    "${EXT}/zlib/trees.c" "${EXT}/zlib/uncompr.c" "${EXT}/zlib/unzip.c"
    "${EXT}/zlib/zip.c" "${EXT}/zlib/zutil.c")
add_library(zlib STATIC ${ZLIB_SRC})
set_target_properties(zlib PROPERTIES ARCHIVE_OUTPUT_DIRECTORY "${XRAY_LIB}")
target_compile_definitions(zlib PRIVATE
    _SILENCE_STDEXT_HASH_DEPRECATION_WARNINGS _WINDLL _SECURE_SCL=0 _ITERATOR_DEBUG_LEVEL=0
    $<$<CONFIG:Release>:DEBUG> $<$<CONFIG:Release>:NDEBUG>)
target_include_directories(zlib PRIVATE "${EXT}/Public" "${EXT}/Public/Zlib")
target_compile_options(zlib PRIVATE /W0 /GR)

#-- RedImage subtree (own property sheet: no engine defines, own includes) -----
set(REDIMAGE "${EXT}/RedImage")

function(redimage_common target)
    set_target_properties(${target} PROPERTIES
        ARCHIVE_OUTPUT_DIRECTORY "${XRAY_LIB}"
        CXX_STANDARD 17)
    target_compile_definitions(${target} PRIVATE _ITERATOR_DEBUG_LEVEL=0)
    target_include_directories(${target} PRIVATE "${REDIMAGE}")
    # note: no /Oi here — nvtt's nvmath.h defines log2f inline, which C2169s
    # against CRT intrinsics; only ispc_texcomp had IntrinsicFunctions on
    target_compile_options(${target} PRIVATE /W0)
endfunction()

# ispc_texcomp: two C++ TUs + ten ISPC-compiled objects (checked-in ispc.exe). The kernels'
# headers are generated into the build folder: ispc writes its own path into them.
set(ISPC_EXE "${REDIMAGE}/ISPC/win/ispc.exe")
set(ISPC_DIR "${REDIMAGE}/ispc_texcomp")
set(ISPC_OUT "${CMAKE_BINARY_DIR}/ispc")
file(MAKE_DIRECTORY "${ISPC_OUT}")
set(ISPC_OBJS "")
set(ISPC_HEADERS "")
foreach(_k kernel kernel_astc)
    set(_objs
        "${ISPC_OUT}/${_k}.obj"      "${ISPC_OUT}/${_k}_sse2.obj" "${ISPC_OUT}/${_k}_sse4.obj"
        "${ISPC_OUT}/${_k}_avx.obj"  "${ISPC_OUT}/${_k}_avx2.obj")
    add_custom_command(
        OUTPUT ${_objs} "${ISPC_OUT}/${_k}_ispc.h"
        COMMAND "${ISPC_EXE}" -O2 "${_k}.ispc"
                -o "${ISPC_OUT}/${_k}.obj"
                -h "${ISPC_OUT}/${_k}_ispc.h"
                --target=sse2,sse4,avx,avx2 --opt=fast-math
        WORKING_DIRECTORY "${ISPC_DIR}"
        MAIN_DEPENDENCY "${ISPC_DIR}/${_k}.ispc"
        DEPENDS "${ISPC_EXE}"
        COMMENT "ISPC ${_k}.ispc")
    list(APPEND ISPC_OBJS ${_objs})
    list(APPEND ISPC_HEADERS "${ISPC_OUT}/${_k}_ispc.h")
endforeach()
set_source_files_properties(${ISPC_OBJS} PROPERTIES EXTERNAL_OBJECT TRUE GENERATED TRUE)
set_source_files_properties(${ISPC_HEADERS} PROPERTIES GENERATED TRUE)

add_library(ispc_texcomp STATIC
    "${ISPC_DIR}/ispc_texcomp.cpp" "${ISPC_DIR}/ispc_texcomp_astc.cpp" ${ISPC_OBJS} ${ISPC_HEADERS})
redimage_common(ispc_texcomp)
target_compile_definitions(ispc_texcomp PRIVATE _MBCS)
target_compile_options(ispc_texcomp PRIVATE /Oi /Gy)
target_include_directories(ispc_texcomp PRIVATE "${ISPC_OUT}")

# nvtt
file(GLOB NVTT_SRC CONFIGURE_DEPENDS
    "${REDIMAGE}/nvtt/bc6h/*.cpp" "${REDIMAGE}/nvtt/bc7/*.cpp" "${REDIMAGE}/nvtt/cuda/*.cpp"
    "${REDIMAGE}/nvtt/nvcore/*.cpp" "${REDIMAGE}/nvtt/nvimage/*.cpp" "${REDIMAGE}/nvtt/nvmath/*.cpp"
    "${REDIMAGE}/nvtt/nvthread/*.cpp" "${REDIMAGE}/nvtt/nvtt/*.cpp" "${REDIMAGE}/nvtt/squish/*.cpp")
add_library(nvtt STATIC ${NVTT_SRC})
redimage_common(nvtt)
target_include_directories(nvtt PRIVATE "${REDIMAGE}/nvtt")

# RedImageTool (Unicode, PCH RedImage.hpp)
file(GLOB REDIMAGETOOL_SRC CONFIGURE_DEPENDS "${REDIMAGE}/RedImageTool/*.cpp")
add_library(RedImageTool STATIC ${REDIMAGETOOL_SRC})
redimage_common(RedImageTool)
target_compile_definitions(RedImageTool PRIVATE _LIB _UNICODE UNICODE)
target_include_directories(RedImageTool PRIVATE "${REDIMAGE}/nvtt")
target_precompile_headers(RedImageTool PRIVATE "${REDIMAGE}/RedImageTool/RedImage.hpp")
target_link_libraries(RedImageTool PUBLIC ispc_texcomp nvtt)
