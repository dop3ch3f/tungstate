#!/bin/sh
# Photograph mock.html scenes in headless Chrome: no window, no focus taken.
# Usage: [EXTRA=bare=1] [SIZE=860,560] scripts/shoot.sh OUTDIR scene [scene...]   (needs `npx vite --port 5174`)
out=$1; shift; mkdir -p "$out"
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
for s in "$@"; do
  "$CHROME" --headless=new --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
    --window-size=${SIZE:-1080,720} --virtual-time-budget=4000 \
    --screenshot="$out/$s.png" "http://localhost:5174/mock.html?scene=$s&$EXTRA" >/dev/null 2>&1
done
ls "$out"
