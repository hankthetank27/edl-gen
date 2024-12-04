#!/bin/bash

set -e

if [ -f .env ]; then
    export $(cat .env | grep -v '#' | xargs)
else
    echo "Error: .env file not found"
    exit 1
fi

if [ -z "$APPLE_ID" ] || [ -z "$APP_SPECIFIC_PASSWORD" ]; then
    echo "Error: APPLE_ID and APP_SPECIFIC_PASSWORD must be set in .env file"
    exit 1
fi


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

echo "Building for x86_64..."
CARGO_TARGET_DIR=target cargo build --release --target x86_64-apple-darwin
echo

echo "Building for aarch64..."
CARGO_TARGET_DIR=target cargo build --release --target aarch64-apple-darwin
echo

DIST_DIR="dist/macos-universal"

mkdir -p $DIST_DIR

echo "Creating universal binary..."
lipo -create \
    "target/x86_64-apple-darwin/release/$PROJECT_NAME" \
    "target/aarch64-apple-darwin/release/$PROJECT_NAME" \
    -output "$DIST_DIR/$PROJECT_NAME"

chmod +x "$DIST_DIR/$PROJECT_NAME"

echo "Universal binary created at $DIST_DIR/$PROJECT_NAME"

echo "Verifying architectures..."
lipo -info "$DIST_DIR/$PROJECT_NAME"

APP_NAME="EDLgen"
BUNDLE_ID="com.hankjackson.edlserver"
VERSION="0.1.0"
COPYRIGHT="© 2024 Hank Jackson. All rights reserved."
ICON_PATH="assets/AppIcon.icns"

APP_BUNDLE="$DIST_DIR/$APP_NAME.app"
CONTENTS_DIR="$APP_BUNDLE/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

echo "Creating app bundle structure..."
rm -rf "$APP_BUNDLE"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

echo "Copying binary..."
cp "$DIST_DIR/$PROJECT_NAME" "$MACOS_DIR/$APP_NAME"
chmod +x "$MACOS_DIR/$APP_NAME"

echo "Creating Info.plist..."
cat > "$CONTENTS_DIR/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>LSMinimumSystemVersion</key>
    <string>10.13</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.utilities</string>
    <key>NSHumanReadableCopyright</key>
    <string>${COPYRIGHT}</string>
</dict>
</plist>
EOF

if [ -f "$ICON_PATH" ]; then
    echo "Copying application icon..."
    cp "$ICON_PATH" "$RESOURCES_DIR/AppIcon.icns"
else
    echo "Warning: Icon file not found at $ICON_PATH"
fi

create_dmg() {
    echo "Creating DMG..."
    DMG_NAME="$DIST_DIR/${APP_NAME}-${VERSION}.dmg"
    
    TMP_DMG_DIR="$DIST_DIR/tmp_dmg"
    rm -rf "$TMP_DMG_DIR"
    mkdir -p "$TMP_DMG_DIR"
    
    cp -R "$APP_BUNDLE" "$TMP_DMG_DIR/"
    
    ln -s /Applications "$TMP_DMG_DIR/Applications"
    
    hdiutil create -volname "$APP_NAME" -srcfolder "$TMP_DMG_DIR" -ov -format UDZO "$DMG_NAME"
    
    rm -rf "$TMP_DMG_DIR"
    echo "DMG created: $DMG_NAME"
}

notarize() {
    echo "Creating ZIP for notarization..."
    ditto -c -k --keepParent "$APP_BUNDLE" "$DIST_DIR/upload.zip"

    echo "Submitting for notarization... (this may take a while)"
    xcrun notarytool submit "$DIST_DIR/upload.zip" \
        --apple-id "$APPLE_ID" \
        --password "$APP_SPECIFIC_PASSWORD" \
        --team-id "$(security find-identity -v -p codesigning | grep "Developer ID Application" | head -1 | sed -n 's/.*(\([A-Z0-9]*\)).*/\1/p')" \
        --wait

    echo "Stapling notarization ticket to app bundle..."
    xcrun stapler staple $APP_BUNDLE

    rm "$DIST_DIR/upload.zip"
    echo "Signed and notarized."
}

# notarize

create_dmg

echo "Your app bundle is ready at $APP_BUNDLE"
