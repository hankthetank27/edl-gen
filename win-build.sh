#!/bin/bash

set -e 

if ! command -v cargo &> /dev/null; then
    echo "cargo is not installed. Please install Rust first."
    exit 1
fi

TEMP_DIR=${TMPDIR:-/tmp}
if [ -d "$TEMP_DIR/asio_sdk" ]; then
    rm -rf "$TEMP_DIR/asio_sdk"
fi

export CPLUS_INCLUDE_PATH="/opt/homebrew/cellar/mingw-w64/12.0.0_1/toolchain-x86_64/x86_64-w64-mingw32/include/"
export CPLUS_INCLUDE_PATH="$CPLUS_INCLUDE_PATH:/opt/homebrew/opt/llvm/include/"

X86_BUILD_DIR="build/win/x86_64-pc-windows-gnu"

mkdir -p $X86_BUILD_DIR

cargo clean
cargo build --target x86_64-pc-windows-gnu -r

cp -p target/x86_64-pc-windows-gnu/release/*.exe $X86_BUILD_DIR
cp -p "/opt/homebrew/opt/mingw-w64/toolchain-x86_64/x86_64-w64-mingw32/lib/libstdc++-6.dll" $X86_BUILD_DIR
cp -p "/opt/homebrew/opt/mingw-w64/toolchain-x86_64/x86_64-w64-mingw32/lib/libgcc_s_seh-1.dll" $X86_BUILD_DIR
cp -p "/opt/homebrew/opt/mingw-w64/toolchain-x86_64/x86_64-w64-mingw32/bin/libwinpthread-1.dll" $X86_BUILD_DIR

echo "64-bit MinGW Windows build complete!"
