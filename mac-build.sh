#!/bin/bash

set -e

if ! command -v cargo &> /dev/null; then
    echo "cargo is not installed. Please install Rust first."
    exit 1
fi

if ! command -v lipo &> /dev/null; then
    echo "lipo is not installed. This script requires Xcode command line tools."
    exit 1
fi

PROJECT_NAME=$(grep -m1 "name" Cargo.toml | cut -d'"' -f2)
if [ -z "$PROJECT_NAME" ]; then
    echo "Could not determine project name from Cargo.toml"
    exit 1
fi

BUILD_DIR=build/macos/
if [ -d $BUILD_DIR ]; then
    rm -rf $BUILD_DIR
fi
mkdir -p $BUILD_DIR

cargo clean

echo "Building for x86_64..."
CARGO_TARGET_DIR=target cargo build --release --target x86_64-apple-darwin
echo

echo "Building for aarch64..."
CARGO_TARGET_DIR=target cargo build --release --target aarch64-apple-darwin
echo

echo "Creating universal binary..."
lipo -create \
    "target/x86_64-apple-darwin/release/$PROJECT_NAME" \
    "target/aarch64-apple-darwin/release/$PROJECT_NAME" \
    -output $BUILD_DIR$PROJECT_NAME

chmod +x "$BUILD_DIR$PROJECT_NAME"

RPATH_OUTPUT=$(otool -l $BUILD_DIR/$PROJECT_NAME | grep -A 2 LC_RPATH)
if [ -z "$RPATH_OUTPUT" ]; then
    echo "Error: RPATH is missing. Required for Sparkle update framework." >&2
    exit 1
else
    echo "RPATH is present:"
    echo "$RPATH_OUTPUT"
fi

echo
echo "MacOS fat binary created at $BUILD_DIR$PROJECT_NAME!"

