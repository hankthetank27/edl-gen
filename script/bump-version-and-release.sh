#!/bin/bash
 
set -euo pipefail
 
# Verify repo is clean
if [ -n "$(git status --porcelain)" ]; then
  printf "Error: repo has uncommitted changes\n"
  exit 1
fi
 
# Get latest version from git tags -- 
# List git tags sorted lexicographically so version numbers sorted correctly
GIT_TAGS=$(git tag --sort=version:refname)
 
# Get last line of output which returns the last tag (most recent version)
GIT_TAG_LATEST=$(echo "$GIT_TAGS" | tail -n 1)
 
# If no tag found, default to v0.0.0
if [ -z "$GIT_TAG_LATEST" ]; then
  GIT_TAG_LATEST="0.0.0"
fi
 
# Strip prefix 'v' from the tag
GIT_TAG_LATEST=$(echo "$GIT_TAG_LATEST" | sed 's/^v//')

# Increment version number
VERSION_TYPE="${1-}"
VERSION_NEXT=""
if [ "$VERSION_TYPE" = "patch" ]; then
  VERSION_NEXT="$(echo "$GIT_TAG_LATEST" | awk -F. '{$NF++; print $1"."$2"."$NF}')"
elif [ "$VERSION_TYPE" = "minor" ]; then
  VERSION_NEXT="$(echo "$GIT_TAG_LATEST" | awk -F. '{$2++; $3=0; print $1"."$2"."$3}')"
elif [ "$VERSION_TYPE" = "major" ]; then
  VERSION_NEXT="$(echo "$GIT_TAG_LATEST" | awk -F. '{$1++; $2=0; $3=0; print $1"."$2"."$3}')"
else
  printf "Error: invalid VERSION_TYPE arg passed, must be 'patch', 'minor' or 'major'\n"
  exit 1
fi
 
# Update Cargo.toml
sed -i "s/^version = .*/version = \"$VERSION_NEXT\"/" Cargo.toml
cargo check
 
git add .
git commit -m "bump version - $VERSION_NEXT"
git tag -a "$VERSION_NEXT" -m "Release: $VERSION_NEXT"
git push origin ci --follow-tags
