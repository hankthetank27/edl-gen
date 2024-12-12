#!/bin/bash

set -e

if ! command -v cargo &> /dev/null; then
    echo "cargo is not installed. Please install Rust first."
    exit 1
fi

PROJECT_NAME=$(grep -m1 "name" Cargo.toml | cut -d'"' -f2)
if [ -z "$PROJECT_NAME" ]; then
    echo "Could not determine project name from Cargo.toml"
    exit 1
fi

if [ -d build/macos ]; then
    rm -rf build/macos/
fi
mkdir -p build/macos/

cargo clean

echo "Building for x86_64..."
CARGO_TARGET_DIR=target cargo build --release --target x86_64-apple-darwin
cp -p target/x86_64-apple-darwin/release/$PROJECT_NAME build/macos/$PROJECT_NAME-x86_64
echo

echo "Building for aarch64..."
CARGO_TARGET_DIR=target cargo build --release --target aarch64-apple-darwin
cp -p target/aarch64-apple-darwin/release/$PROJECT_NAME build/macos/$PROJECT_NAME-aarch64
echo

echo "MacOS x86_64-apple-darwin and aarch64-apple-darwin builds are ready at /build/macos/"
