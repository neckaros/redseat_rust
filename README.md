Initial development

For AVIF support
On x86 install NASM
https://github.com/xiph/rav1e/#dependency-nasm

Install Clang
libheif

Windows:
// optional install visual studio C++
winget install LLVM.LLVM
git clone https://github.com/Microsoft/vcpkg.git
.\vcpkg\bootstrap-vcpkg.bat
Add vcpkg to env variables
vcpkg install libde265:x64-windows
vcpkg install x265:x64-windows
vcpkg install libheif:x64-windows


vcpkg install dav1d
cargo vcpkg -v build

Mac:
 brew install llvm


Path of configuration for docker volume:
/root/.config/redseat
