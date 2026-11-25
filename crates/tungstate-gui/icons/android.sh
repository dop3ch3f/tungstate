#!/bin/sh
# Builds the Android launcher icons from android-foreground.svg and android-legacy.svg.
# `tauri icon` would overwrite them with the desktop mark, whose dark plate One UI
# crops into a black frame, so run this after it. Needs rsvg-convert and magick.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
res="$here/../gen/android/app/src/main/res"
for pair in mdpi:1 hdpi:1.5 xhdpi:2 xxhdpi:3 xxxhdpi:4; do
  dir="$res/mipmap-${pair%%:*}"; k=${pair#*:}
  fg=$(echo "108 * $k / 1" | bc); legacy=$(echo "48 * $k / 1" | bc)
  rsvg-convert -w "$fg" -h "$fg" "$here/android-foreground.svg" -o "$dir/ic_launcher_foreground.png"
  rsvg-convert -w "$legacy" -h "$legacy" "$here/android-legacy.svg" -o "$dir/ic_launcher.png"
  # Round: the same face cut to a circle.
  magick "$dir/ic_launcher.png" \( -size "${legacy}x${legacy}" xc:none -fill white \
    -draw "circle $((legacy / 2)),$((legacy / 2)) $((legacy / 2)),$((legacy * 4 / 100))" \) \
    -compose DstIn -composite "$dir/ic_launcher_round.png"
done
