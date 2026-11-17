#!/bin/zsh
RELEASE_ROOT=build
APP_NAME="Bruces Smart Home"
EXE_NAME="bruce-smart-home"

function create_dmg {
    readonly arch=${1:?"The arch triplet must be specified."}
    readonly source=${2:?"The exe file path must be specified."}
    readonly title=${3:?"The title must be specified."}

    if ! test -d "$source"
    then
        echo Source folder $source does not exist.
        exit 1
    fi
    bytes=`du -k  target/release/bruce-smart-home | sed -e 's/[ \t].*//'`
    disk_size=$(($bytes * 2))

    # disk_size=1024

    image_file="$RELEASE_ROOT/${title}.dmg"
    temp_image_file="$RELEASE_ROOT/pack.temp.dmg"

    rm -f $temp_image_file
    echo creating disk image pack.temp.dmg of size $disk_size kbytes
    hdiutil create -verbose -srcfolder "${source}" -volname "${title}" -fs HFS+ -fsargs "-c c=64,a=16,e=16" -format UDRW -size ${disk_size}k $temp_image_file

    echo mounting disk image...
    device=$(hdiutil attach -readwrite -noverify -noautoopen "$temp_image_file" | egrep '^/dev/' | sed 1q | awk '{print $1}')
    sleep 5
    echo Copy background image...
    mkdir "/Volumes/${title}/.background"
    cp release/dmg-background.png "/Volumes/${title}/.background"

    echo Set finder properties...
    echo '
    tell application "Finder"
        tell disk "'${title}'"
            open
            set current view of container window to icon view
            set toolbar visible of container window to false
            set statusbar visible of container window to false
            set the bounds of container window to {400, 100, 885, 430}
            set theViewOptions to the icon view options of container window
            set arrangement of theViewOptions to not arranged
            set icon size of theViewOptions to 72
            set background picture of theViewOptions to file ".background:'dmg-background.png'"
            make new alias file at container window to POSIX file "/Applications" with properties {name:"Applications"}
            set position of item "'${APP_NAME}.app'" of container window to {100, 100}
            set position of item "Applications" of container window to {375, 100}
            update without registering applications
            delay 5
            close
        end tell
    end tell
    ' | osascript

    echo Finalize...
    chmod -Rf go-w /Volumes/"${title}"
    sync
    sync
    echo Detatch...
    hdiutil detach ${device}
    echo Convert...
    hdiutil convert "$temp_image_file" -format UDZO -imagekey zlib-level=9 -o "${image_file}"
    rm -f $temp_image_file 
    echo Done.
}

function create_app {
    readonly arch=${1:?"The arch triplet must be specified."}

    mkdir -p "$RELEASE_ROOT/$arch/$APP_NAME.app/Contents/MacOS"
    mkdir -p "$RELEASE_ROOT/$arch/$APP_NAME.app/Contents/Resources"
    cat > "$RELEASE_ROOT/$arch/$APP_NAME.app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDisplayName</key>
	<string>$APP_NAME</string>
    <key>CFBundleIconFile</key>
    <string>app-icon.icns</string>
</dict>
</plist>
EOF
    chmod 444 "$RELEASE_ROOT/$arch/$APP_NAME.app/Contents/Info.plist"
    cat > "$RELEASE_ROOT/$arch/$APP_NAME.app/Contents/MacOS/$APP_NAME" <<EOF
#!/usr/bin/osascript
set AppleScript's text item delimiters to "/"
set myPath to {"clear && exec "} as text & (quoted form of ((text items 1 through -2 of (POSIX path of (path to me as text))) & {"Bruces Smart Home"} as text))

tell application "Terminal"
	activate
	do script myPath
	set the number of columns of window 1 to 150
end tell
EOF
    chmod 555 "$RELEASE_ROOT/$arch/$APP_NAME.app/Contents/MacOS/$APP_NAME"
    cp release/app-icon.icns "$RELEASE_ROOT/$arch/$APP_NAME.app/Contents/Resources"
    chmod 444 "$RELEASE_ROOT/$arch/$APP_NAME.app/Contents/Resources/app-icon.icns"
}

VERSION=`grep "^version = " Cargo.toml | sed -e 's/[^"]*"\([^"]*\)"/\1/'`
echo Gathering build artefacts for version $VERSION...
rm -rf build
mkdir -p build

# tempfile=`mktemp /tmp/bruce-smart-home-XXXXXXXX`
# scp $BUILDER_X86_64_APPLE_DARWIN:git/bruce-smart-home/target/release/bruce-smart-home $tempfile

arch=aarch64-apple-darwin
create_app $arch
cp target/dx/bruce-smart-home "$RELEASE_ROOT/$arch/$APP_NAME.app/Contents/MacOS"
cp -r assets "$RELEASE_ROOT/$arch/$APP_NAME.app/Contents/Resources/assets"
create_dmg $arch "$RELEASE_ROOT/$arch" "bruce-smart-home-$VERSION-$arch"


arch=x86_64-apple-darwin
create_app $arch
scp $BUILDER_X86_64_APPLE_DARWIN:git/bruce-smart-home/target/release/bruce-smart-home "$RELEASE_ROOT/$arch/$APP_NAME.app/Contents/MacOS"
create_dmg $arch "$RELEASE_ROOT/$arch" "bruce-smart-home-$VERSION-$arch"

cp target/x86_64-pc-windows-msvc/release/bruce-smart-home.exe build/bruce-smart-home-$VERSION-x86_64-pc-windows-msvc.exe
cp target/aarch64-pc-windows-msvc/release/bruce-smart-home.exe build/bruce-smart-home-$VERSION-aarch64-pc-windows-msvc.exe
