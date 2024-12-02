#!/bin/bash

export CPLUS_INCLUDE_PATH="$CPLUS_INCLUDE_PATH:/opt/homebrew/Cellar/mingw-w64/12.0.0_1/toolchain-x86_64/x86_64-w64-mingw32/include/"
export CPLUS_INCLUDE_PATH="$CPLUS_INCLUDE_PATH:/opt/homebrew/opt/llvm/include/"

mkdir -p dist/x86_64-pc-windows-gnu

cargo clean
cargo build --target x86_64-pc-windows-gnu -r

cp target/x86_64-pc-windows-gnu/release/*.exe dist/x86_64-pc-windows-gnu

cp "/opt/homebrew/opt/mingw-w64/toolchain-x86_64/x86_64-w64-mingw32/lib/libstdc++-6.dll" dist/x86_64-pc-windows-gnu
cp "/opt/homebrew/opt/mingw-w64/toolchain-x86_64/x86_64-w64-mingw32/lib/libgcc_s_seh-1.dll" dist/x86_64-pc-windows-gnu
cp "/opt/homebrew/opt/mingw-w64/toolchain-x86_64/x86_64-w64-mingw32/bin/libwinpthread-1.dll" dist/x86_64-pc-windows-gnu

echo "Build complete!"
