#!/usr/bin/env bash
# Build a release package of Snowballz into dist/.
#
#   tools/make-release.sh linux   -> dist/snowballz-<version>-linux-x64.tar.gz
#                                    (+ dist/snowballz-<version>-x86_64.AppImage
#                                    when appimagetool is installed or
#                                    $APPIMAGETOOL points to it)
#   tools/make-release.sh macos   -> dist/snowballz-<version>-macos.zip
#   tools/make-release.sh relay [target] [arch]
#                                 -> dist/snowballz-relay-<version>-linux-<arch>.tar.gz
#                                    (the relay server for game codes; target
#                                    e.g. x86_64-unknown-linux-musl, a static
#                                    program that runs on any Linux)
#
# Linux: bundles the system's SDL2 (install libsdl2-2.0-0 first) as a
# fallback in lib/.  macOS: builds a universal binary when both Apple
# targets are installed (rustup target add x86_64-apple-darwin
# aarch64-apple-darwin) and puts SDL2.framework in the app: from
# $SDL2_FRAMEWORK if set, otherwise downloaded (SDL $SDL2_VERSION).
# Windows: use tools\make-release.ps1.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(tr -d ' \r\n' < VERSION)
SDL2_VERSION=${SDL2_VERSION:-2.32.10}
case "$(uname -s)" in
    Darwin) default=macos ;;
    *) default=linux ;;
esac
PLATFORM=${1:-$default}
mkdir -p dist

# game files shared by every package: $1 = destination folder
copy_game_files() {
    cp -R data maps plugins "$1/"
    cp icon.png wallpaper.png README.md LICENSE "$1/"
    mkdir -p "$1/licenses"
    cp tools/licenses/SDL2.txt "$1/licenses/"
}

play_txt() { # $1 = platform, $2 = destination file
    sed "s/@VERSION@/$VERSION/" "tools/release/PLAY-$1.txt" > "$2"
}

case "$PLATFORM" in
linux)
    cargo build --release --locked
    name="snowballz-$VERSION-linux-x64"
    out="dist/$name"
    rm -rf "$out"
    mkdir -p "$out/lib"
    cp target/release/snowballz "$out/"
    copy_game_files "$out"
    cp tools/release/snowballz-512.png "$out/snowballz.png"
    cp tools/release/install-desktop-entry.sh "$out/"
    chmod +x "$out/install-desktop-entry.sh"
    play_txt linux "$out/PLAY.txt"
    sdl=$(/sbin/ldconfig -p 2>/dev/null | awk '/libSDL2-2\.0\.so\.0 .*x86-64/ {print $NF; exit}')
    if [ -n "$sdl" ]; then
        cp -L "$sdl" "$out/lib/libSDL2-2.0.so.0"
    else
        echo "warning: no system SDL2 found; the package will need the player's SDL2" >&2
        rmdir "$out/lib"
    fi
    tar -C dist -czf "dist/$name.tar.gz" "$name"
    echo "dist/$name.tar.gz"
    # the same folder as an AppImage
    appdir=dist/Snowballz.AppDir
    rm -rf "$appdir"
    cp -R "$out" "$appdir"
    rm -f "$appdir/install-desktop-entry.sh"
    cp tools/release/AppRun "$appdir/AppRun"
    chmod +x "$appdir/AppRun"
    cp tools/release/snowballz.desktop "$appdir/snowballz.desktop"
    cp tools/release/snowballz-512.png "$appdir/.DirIcon"
    tool=${APPIMAGETOOL:-$(command -v appimagetool || true)}
    if [ -n "$tool" ]; then
        image="dist/snowballz-$VERSION-x86_64.AppImage"
        if ARCH=x86_64 "$tool" "$appdir" "$image"; then
            echo "$image"
        else
            echo "warning: appimagetool failed; no AppImage this time" >&2
        fi
    else
        echo "note: appimagetool not found, so no AppImage (the AppDir is in $appdir)" >&2
    fi
    ;;
macos)
    targets=$(rustup target list --installed 2>/dev/null || true)
    if echo "$targets" | grep -q aarch64-apple-darwin && echo "$targets" | grep -q x86_64-apple-darwin; then
        cargo build --release --locked --target aarch64-apple-darwin
        cargo build --release --locked --target x86_64-apple-darwin
        bin=dist/snowballz-universal
        lipo -create -output "$bin" \
            target/aarch64-apple-darwin/release/snowballz \
            target/x86_64-apple-darwin/release/snowballz
    else
        echo "note: building for this Mac's CPU only (add both Apple targets for a universal app)" >&2
        cargo build --release --locked
        bin=target/release/snowballz
    fi
    app=dist/Snowballz.app
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Frameworks" "$app/Contents/Resources"
    cp "$bin" "$app/Contents/MacOS/snowballz"
    sed "s/@VERSION@/$VERSION/g" tools/release/Info.plist > "$app/Contents/Info.plist"
    cp tools/release/snowballz.icns "$app/Contents/Resources/"
    copy_game_files "$app/Contents/Resources"
    # SDL2.framework
    if [ -n "${SDL2_FRAMEWORK:-}" ]; then
        cp -R "$SDL2_FRAMEWORK" "$app/Contents/Frameworks/"
    else
        dmg="dist/SDL2-$SDL2_VERSION.dmg"
        [ -f "$dmg" ] || curl -fL -o "$dmg" \
            "https://github.com/libsdl-org/SDL/releases/download/release-$SDL2_VERSION/SDL2-$SDL2_VERSION.dmg"
        mnt=$(mktemp -d)
        hdiutil attach -nobrowse -readonly -mountpoint "$mnt" "$dmg" >/dev/null
        # the macOS slice of an xcframework, or a plain framework
        fw=$(find "$mnt" -type d -name SDL2.framework -path '*macos*' | head -n1)
        [ -n "$fw" ] || fw=$(find "$mnt" -maxdepth 3 -type d -name SDL2.framework | head -n1)
        if [ -z "$fw" ]; then
            hdiutil detach "$mnt" >/dev/null
            echo "error: no SDL2.framework in $dmg" >&2
            exit 1
        fi
        cp -R "$fw" "$app/Contents/Frameworks/"
        hdiutil detach "$mnt" >/dev/null
    fi
    # ad-hoc signature (Apple Silicon needs one; this is not Apple notarization)
    codesign --force --deep --sign - "$app"
    name="snowballz-$VERSION-macos"
    rm -rf "dist/$name"
    mkdir -p "dist/$name"
    mv "$app" "dist/$name/"
    play_txt macos "dist/$name/PLAY.txt"
    cp README.md LICENSE "dist/$name/"
    (cd dist && rm -f "$name.zip" && ditto -c -k --keepParent "$name" "$name.zip")
    echo "dist/$name.zip"
    ;;
relay)
    target=${2:-}
    arch=${3:-x64}
    if [ -n "$target" ]; then
        cargo build --release --locked --bin snowballz-relay --target "$target"
        bin="target/$target/release/snowballz-relay"
    else
        cargo build --release --locked --bin snowballz-relay
        bin="target/release/snowballz-relay"
    fi
    name="snowballz-relay-$VERSION-linux-$arch"
    out="dist/$name"
    rm -rf "$out"
    mkdir -p "$out"
    cp "$bin" "$out/"
    strip "$out/snowballz-relay" 2>/dev/null || true
    sed "s/@VERSION@/$VERSION/g" tools/release/RELAY-SERVER.txt > "$out/README.txt"
    cp tools/release/snowballz-relay.service LICENSE "$out/"
    tar -C dist -czf "dist/$name.tar.gz" "$name"
    echo "dist/$name.tar.gz"
    ;;
*)
    echo "usage: $0 [linux|macos|relay]   (Windows: tools\\make-release.ps1)" >&2
    exit 2
    ;;
esac
