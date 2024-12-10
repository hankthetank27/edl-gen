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

cargo clean
echo "Building for x86_64..."
CARGO_TARGET_DIR=target cargo build --release --target x86_64-apple-darwin
echo

echo "Building for aarch64..."
CARGO_TARGET_DIR=target cargo build --release --target aarch64-apple-darwin
echo

mkdir -p build/macos/
cp target/x86_64-apple-darwin/release/$PROJECT_NAME build/macos/
cp target/aarch64-apple-darwin/release/$PROJECT_NAME build/macos/

echo "MacOS x86_64-apple-darwin and aarch64-apple-darwin builds are ready at /build/macos/"
