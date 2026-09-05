#!/usr/bin/env bash
set -euo pipefail

libheif_version="1.23.3"
libheif_sha256="79e1f66059e55728e541b671f347e3fa787cedeb61170f4e75efe8aaee6ef59e"
libheif_build_dir="$(mktemp -d)"

cleanup() {
    rm -rf "${libheif_build_dir}"
}
trap cleanup EXIT

curl --fail --location \
    --output "${libheif_build_dir}/libheif.tar.gz" \
    "https://github.com/strukturag/libheif/archive/refs/tags/v${libheif_version}.tar.gz"

printf '%s  %s\n' \
    "${libheif_sha256}" \
    "${libheif_build_dir}/libheif.tar.gz" | sha256sum --check

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
    -DCMAKE_INSTALL_PREFIX=/usr/local \
    -DCMAKE_INSTALL_LIBDIR=lib \
    -DBUILD_SHARED_LIBS=ON \
    -DBUILD_TESTING=OFF \
    -DENABLE_PLUGIN_LOADING=OFF \
    -DWITH_AOM_DECODER=ON \
    -DWITH_AOM_DECODER_PLUGIN=OFF \
    -DWITH_AOM_ENCODER=ON \
    -DWITH_AOM_ENCODER_PLUGIN=OFF \
    -DWITH_DAV1D=OFF \
    -DWITH_EXAMPLES=OFF \
    -DWITH_LIBDE265=ON \
    -DWITH_LIBDE265_PLUGIN=OFF \
    -DWITH_LIBSHARPYUV=OFF \
    -DWITH_OpenH264_DECODER=OFF \
    -DWITH_X264=OFF \
    -DWITH_X265=ON \
    -DWITH_X265_PLUGIN=OFF

cmake --build "${libheif_build_dir}/build" --parallel
cmake --install "${libheif_build_dir}/build"

if command -v ldconfig >/dev/null 2>&1; then
    ldconfig
fi
