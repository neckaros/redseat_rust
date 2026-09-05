#!/usr/bin/env bash
set -euo pipefail

libheif_version="1.23.3"
libheif_sha256="79e1f66059e55728e541b671f347e3fa787cedeb61170f4e75efe8aaee6ef59e"
libde265_version="1.1.1"
libde265_sha256="5b4fac677018e6074196e8f9889f3e4a5310e46afbf22a893f620d4e24d3510e"
native_prefix="${REDSEAT_NATIVE_PREFIX:-/usr/local}"
libheif_build_dir="$(mktemp -d)"

cleanup() {
    rm -rf "${libheif_build_dir}"
}
trap cleanup EXIT

verify_sha256() {
    local expected="$1"
    local archive="$2"

    if command -v sha256sum >/dev/null 2>&1; then
        printf '%s  %s\n' "${expected}" "${archive}" | sha256sum --check
    elif command -v shasum >/dev/null 2>&1; then
        printf '%s  %s\n' "${expected}" "${archive}" | shasum --algorithm 256 --check
    else
        echo "sha256sum or shasum is required to verify source archives" >&2
        return 1
    fi
}

curl --fail --location \
    --output "${libheif_build_dir}/libde265.tar.gz" \
    "https://github.com/strukturag/libde265/archive/refs/tags/v${libde265_version}.tar.gz"

verify_sha256 \
    "${libde265_sha256}" \
    "${libheif_build_dir}/libde265.tar.gz"

mkdir "${libheif_build_dir}/libde265-source"
tar --extract \
    --gzip \
    --file "${libheif_build_dir}/libde265.tar.gz" \
    --strip-components 1 \
    --directory "${libheif_build_dir}/libde265-source"

# ENABLE_DECODER controls the optional dec265 CLI; the decoder library is always built.
cmake \
    -S "${libheif_build_dir}/libde265-source" \
    -B "${libheif_build_dir}/libde265-build" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="${native_prefix}" \
    -DCMAKE_INSTALL_LIBDIR=lib \
    -DBUILD_SHARED_LIBS=OFF \
    -DENABLE_DECODER=OFF \
    -DENABLE_ENCODER=OFF \
    -DENABLE_SDL=OFF

cmake --build "${libheif_build_dir}/libde265-build" --parallel
cmake --install "${libheif_build_dir}/libde265-build"

export PKG_CONFIG_PATH="${native_prefix}/lib/pkgconfig${PKG_CONFIG_PATH:+:${PKG_CONFIG_PATH}}"

curl --fail --location \
    --output "${libheif_build_dir}/libheif.tar.gz" \
    "https://github.com/strukturag/libheif/archive/refs/tags/v${libheif_version}.tar.gz"

verify_sha256 \
    "${libheif_sha256}" \
    "${libheif_build_dir}/libheif.tar.gz"

mkdir "${libheif_build_dir}/source"
tar --extract \
    --gzip \
    --file "${libheif_build_dir}/libheif.tar.gz" \
    --strip-components 1 \
    --directory "${libheif_build_dir}/source"

cmake \
    -S "${libheif_build_dir}/source" \
    -B "${libheif_build_dir}/build" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="${native_prefix}" \
    -DCMAKE_INSTALL_LIBDIR=lib \
    -DCMAKE_PREFIX_PATH="${native_prefix}" \
    -DBUILD_DOCUMENTATION=OFF \
    -DBUILD_SHARED_LIBS=OFF \
    -DBUILD_TESTING=OFF \
    -DENABLE_PLUGIN_LOADING=OFF \
    -DWITH_AOM_DECODER=OFF \
    -DWITH_AOM_ENCODER=OFF \
    -DWITH_DAV1D=OFF \
    -DWITH_EXAMPLES=OFF \
    -DWITH_GDK_PIXBUF=OFF \
    -DWITH_LIBDE265=ON \
    -DWITH_LIBDE265_PLUGIN=OFF \
    -DWITH_LIBSHARPYUV=OFF \
    -DWITH_OpenH264_DECODER=OFF \
    -DWITH_X264=OFF \
    -DWITH_X265=OFF

cmake --build "${libheif_build_dir}/build" --parallel
cmake --install "${libheif_build_dir}/build"
