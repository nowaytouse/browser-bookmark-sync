#!/bin/bash
set -euo pipefail

fail() { printf 'Error: %s\n' "$*" >&2; exit 1; }

[[ $# -eq 0 ]] || fail 'This script takes no arguments.'
[[ "$(uname -s)" == Darwin ]] || fail 'This app can only be built on macOS.'
root=$(cd "$(dirname "$0")/../.." && pwd -P)
target="$root/target"
app="$root/Bookmark Cleaner.app"
bundle_id=local.bsync.bookmark-review
stage=
candidate=
candidate_inode=
new_moved=0
complete=0

for tool in cargo sips iconutil codesign plutil pgrep mktemp stat; do
    command -v "$tool" >/dev/null || fail "Required tool is unavailable: $tool"
done
[[ ! -L "$target" ]] || fail 'The repository target directory must not be a symlink.'
mkdir -p "$target"
[[ "$(stat -f %d "$root")" == "$(stat -f %d "$target")" ]] || fail 'The target directory must be on the repository filesystem for atomic app replacement.'
lock="$target/.bookmark-cleaner-build.lock"
mkdir "$lock" 2>/dev/null || fail "Another build is active, or a previous build left $lock; inspect it before retrying."

cleanup() {
    status=$?
    trap - EXIT
    set +e
    keep_stage=0
    if [[ -n "$stage" && $complete -eq 0 ]]; then
        if [[ $new_moved -eq 1 && ! -e "$candidate" && ! -L "$candidate" ]]; then
            if [[ -d "$app" && ! -L "$app" && "$(stat -f %i "$app")" == "$candidate_inode" ]]; then
                mv "$app" "$stage/rejected.app" || keep_stage=1
            elif [[ -e "$app" || -L "$app" ]]; then
                keep_stage=1
            fi
        fi
        if [[ -e "$stage/previous.app" || -L "$stage/previous.app" ]]; then
            if [[ $keep_stage -eq 0 && ! -e "$app" && ! -L "$app" ]] && mv "$stage/previous.app" "$app"; then
                printf 'Restored the previous Bookmark Cleaner.app.\n' >&2
            else
                keep_stage=1
            fi
        fi
    fi
    if [[ $keep_stage -eq 1 ]]; then
        status=1
        printf 'Rollback needs attention; recovery files preserved at %s\n' "$stage" >&2
    fi
    if [[ -n "$stage" && $keep_stage -eq 0 ]]; then
        case "$stage" in
            "$target"/.bookmark-cleaner.??????)
                if [[ -d "$stage" && ! -L "$stage" ]]; then
                    rm -rf -- "$stage" || status=1
                else
                    printf 'Refusing to clean an unexpected staging path: %s\n' "$stage" >&2
                    status=1
                fi
                ;;
            *) printf 'Refusing to clean an unexpected staging path: %s\n' "$stage" >&2; status=1 ;;
        esac
    fi
    rmdir "$lock" || status=1
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
stage=$(mktemp -d "$target/.bookmark-cleaner.XXXXXX")

require_stopped() {
    if pgrep -x bookmark-review >/dev/null; then
        fail 'Quit Bookmark Cleaner before building; no running app will be replaced.'
    else
        status=$?
        [[ $status -eq 1 ]] || fail 'Could not determine whether Bookmark Cleaner is running.'
    fi
}

check_existing() {
    [[ ! -L "$app" ]] || fail 'Refusing to replace a symlink at Bookmark Cleaner.app.'
    if [[ -e "$app" ]]; then
        [[ -d "$app" && -f "$app/Contents/Info.plist" && ! -L "$app/Contents/Info.plist" ]] || fail 'Existing Bookmark Cleaner.app is not a normal app bundle.'
        actual_id=$(plutil -extract CFBundleIdentifier raw -expect string "$app/Contents/Info.plist")
        [[ "$actual_id" == "$bundle_id" ]] || fail "Refusing to replace an app with bundle identifier: $actual_id"
    fi
}

# These assertions also protect the fixed GUI/engine/icon packaging contract.
check_bundle() {
    bundle=$1
    plutil -lint "$bundle/Contents/Info.plist"
    [[ "$(plutil -extract CFBundleIdentifier raw -expect string "$bundle/Contents/Info.plist")" == "$bundle_id" ]] || fail 'Unexpected bundle identifier.'
    [[ "$(plutil -extract CFBundleExecutable raw -expect string "$bundle/Contents/Info.plist")" == bookmark-review ]] || fail 'Unexpected app executable.'
    [[ "$(plutil -extract CFBundleIconFile raw -expect string "$bundle/Contents/Info.plist")" == AppIcon ]] || fail 'Info.plist must name AppIcon.'
    for executable in "$bundle/Contents/MacOS/bookmark-review" "$bundle/Contents/Resources/bsync"; do
        [[ -f "$executable" && ! -L "$executable" && -x "$executable" ]] || fail "Missing executable: $executable"
    done
    [[ -s "$bundle/Contents/Resources/AppIcon.icns" ]] || fail 'The app icon is missing or empty.'
    codesign --verify --strict "$bundle/Contents/Resources/bsync"
    codesign --verify --deep --strict "$bundle"
}

require_stopped
check_existing
cd "$root"
cargo build --locked --offline --release --features native-gui --bin bookmark-review --bin browser-bookmark-sync -j2 --target-dir "$target"

candidate="$stage/Bookmark Cleaner.app"
resources="$candidate/Contents/Resources"
iconset="$stage/AppIcon.iconset"
mkdir -p "$candidate/Contents/MacOS" "$resources" "$iconset"
cp "$root/packaging/macos/Info.plist" "$candidate/Contents/Info.plist"
cp "$target/release/bookmark-review" "$candidate/Contents/MacOS/bookmark-review"
cp "$target/release/browser-bookmark-sync" "$resources/bsync"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$root/assets/branding/bookmark-bsync-cat-card-3d.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
    sips -z "$((size * 2))" "$((size * 2))" "$root/assets/branding/bookmark-bsync-cat-card-3d.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil --convert icns --output "$resources/AppIcon.icns" "$iconset"
codesign --force --sign - --timestamp=none "$resources/bsync"
codesign --force --sign - --timestamp=none "$candidate"
check_bundle "$candidate"
candidate_inode=$(stat -f %i "$candidate")

require_stopped
check_existing
if [[ -e "$app" ]]; then
    mv "$app" "$stage/previous.app"
fi
[[ ! -e "$app" && ! -L "$app" ]] || fail 'The destination changed during app replacement.'
new_moved=1
mv "$candidate" "$app"
check_bundle "$app"
complete=1
printf 'Built and verified %s\n' "$app"
