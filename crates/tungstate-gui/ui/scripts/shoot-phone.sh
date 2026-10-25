#!/bin/sh
# Photograph phone.html at a real phone size, 390x844 points at 2x, in
# headless Chrome. Chrome will not make a window narrower than 500px, so the
# harness holds the app to 390 wide and the shot is cropped to it.
# Usage: scripts/shoot-phone.sh OUTDIR name "query" [name "query" ...]
#   (needs `npx vite --port 5175`)
out=$1; shift; mkdir -p "$out"
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
while [ $# -ge 2 ]; do
  name=$1; query=$2; shift 2
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=2 \
    --window-size=500,844 --virtual-time-budget=6000 --screenshot="$out/$name.png" \
    "http://localhost:5175/phone.html?phone&$query" >/dev/null 2>&1
  magick "$out/$name.png" -crop 780x1688+0+0 +repage "$out/$name.png"
done
ls "$out"
