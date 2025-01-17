#!/bin/bash

set -e 

if [ -d "./output" ]; then
    rm -rf "./output"
fi

if ! command -v conveyor &> /dev/null; then
    echo "Conveyor is not installed"
    exit 1
fi

# export GITHUB_TOKEN
if [ -f .env ]; then
    export $(grep -v '^#' .env | xargs)
fi

export CARGO_VERSION=$(awk -F ' = ' '$1 ~ /version/ { gsub(/[\"]/, "", $2); printf("%s",$2) }' Cargo.toml)

echo "Compiling MacOS binary..."
./script/mac-build.sh
echo
echo "Compiling Windows binary..."
./script/win-build.sh
conveyor make copied-site
echo "Release deployed as draft on GitHub @ https://github.com/hankthetank27/edl-gen/releases"
