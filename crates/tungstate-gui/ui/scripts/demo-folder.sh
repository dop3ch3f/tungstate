#!/bin/sh
# Build a set of folders worth pointing the window at.
#
# Every screen in the folder half needs a folder in a particular condition to
# be reachable at all: rules that will not parse, rules that never settle, a
# folder already tidy, a folder whose shape has to be read back out of it. This
# makes one root per condition under $TMPDIR, so nothing lands in the repo and
# there is nothing to gitignore.
#
# Deterministic: every name, size and timestamp comes from a counter, never
# from $RANDOM, so two runs produce identical trees and two screenshots taken a
# week apart are comparable.
#
# Usage:  sh scripts/demo-folder.sh [--clean]
set -eu

DEMO="${TUNGSTATE_DEMO:-${TMPDIR:-/tmp}/tungstate-demo}"
DEMO="${DEMO%/}"
REPO=$(cd "$(dirname "$0")/../../../.." && pwd)

if [ "${1:-}" = "--clean" ]; then
	chmod -R u+rwX "$DEMO" 2>/dev/null || true
	rm -rf "$DEMO"
	echo "removed $DEMO"
	exit 0
fi

if [ -e "$DEMO" ]; then
	echo "error: $DEMO already exists. Run with --clean first." >&2
	exit 1
fi

# --- making files -----------------------------------------------------------

# Magic bytes, because mime is sniffed from the first bytes and only falls back
# to the extension for text. An empty `.jpg` is `application/octet-stream` and
# matches none of the image rules, so a demo folder of empty files would
# quietly exercise nothing. `infer` needs four bytes for PNG and the `ftyp` box
# at offset four for MP4; the rest is padding to look like the real thing.
magic() {
	case "$1" in
	jpg | jpeg) printf '\377\330\377\340\000\020JFIF\000\001\001\001\000\110\000\110\000\000' ;;
	png) printf '\211PNG\r\n\032\n\000\000\000\015IHDR' ;;
	heic) printf '\000\000\000\030ftypheic\000\000\000\000heicmif1' ;;
	mp4) printf '\000\000\000\030ftypmp42\000\000\000\000mp42isom' ;;
	mov) printf '\000\000\000\024ftypqt  \000\000\000\000qt  ' ;;
	mp3) printf 'ID3\003\000\000\000\000\000\000' ;;
	pdf) printf '%%PDF-1.4\n%%\342\343\317\323\n' ;;
	zip) printf 'PK\003\004\024\000\000\000\010\000' ;;
	dmg) printf 'PK\003\004\024\000\000\000\010\000' ;;
	txt | md | csv | log) printf 'Nothing in this file matters except that it is text.\n' ;;
	*) printf 'demo\n' ;;
	esac
}

# A file of a given apparent size, sparse past its header.
#
# `dd count=0 seek=N` extends without writing, so a 1.4GB file costs one block.
# That is what makes the `>1GB` size band reachable on a laptop: the engine
# reads size from metadata, and a hole is as big as a hole says it is.
mkf() {
	path=$1
	size=$2
	stamp=$3
	ext=${path##*.}
	mkdir -p "$(dirname "$path")"
	magic "$ext" >"$path"
	head=$(wc -c <"$path" | tr -d ' ')
	if [ "$size" -gt "$head" ]; then
		dd if=/dev/zero of="$path" bs=1 count=0 seek="$size" 2>/dev/null
	fi
	touch -t "$stamp" "$path"
}

echo "building $DEMO"
mkdir -p "$DEMO"

# --- 1. the shape the user actually keeps -----------------------------------
# Named so the media layout has something to recognise: WhatsApp files carry
# `-WA####`, Telegram's carry `photo_`/`video_` and a date, screenshots say so.
# The year comes from the mtime, so these are stamped across four years.

messy_media() {
	root=$1
	i=0
	for year in 2023 2024 2025 2026; do
		for month in 1 2 3 4 5 6 7 8; do
			mon=$(printf '%02d' "$month")
			for n in 1 2 3 4 5 6; do
				i=$((i + 1))
				day=$(printf '%02d' $(((i % 27) + 1)))
				at="${year}${mon}${day}1200"
				case $((i % 6)) in
				0) mkf "$root/IMG-${year}${mon}${day}-WA0${i}.jpg" $((900000 + i * 1000)) "$at" ;;
				1) mkf "$root/VID-${year}${mon}${day}-WA1${i}.mp4" $((180000000 + i * 1000)) "$at" ;;
				2) mkf "$root/photo_${year}-${mon}-${day}_18-22-${n}${i}.jpg" $((700000 + i * 1000)) "$at" ;;
				3) mkf "$root/video_${year}-${mon}-${day}_09-04-${n}${i}.mp4" $((620000000 + i * 1000)) "$at" ;;
				4) mkf "$root/Screenshot ${year}-${mon}-${day} at 14.03.${n}${i}.png" $((240000 + i * 1000)) "$at" ;;
				5) mkf "$root/DSC${year}${i}.jpg" $((1400000000 + i * 1000)) "$at" ;;
				esac
			done
		done
	done
}

messy_media "$DEMO/real-shape"

# --- 2. a flat downloads folder, with the awkward cases ---------------------

D="$DEMO/messy-downloads"
mkdir -p "$D"
i=0
for name in invoice-march receipt-2024 contract-draft notes-from-call \
	minutes budget-q3 proposal-v2 readme changelog spec; do
	i=$((i + 1))
	mkf "$D/$name.pdf" $((40000 + i * 900)) "20260$(((i % 9) + 1))121030"
done
k=0
for name in holiday sunset kitchen garden bridge; do
	i=$((i + 1))
	k=$((k + 1))
	mkf "$D/$name.jpg" $((2400000 + i * 7000)) "2026060${k}0930"
done
k=0
for name in talk demo walkthrough; do
	i=$((i + 1))
	k=$((k + 1))
	mkf "$D/$name.mp4" $((320000000 + i * 5000)) "2026070${k}1130"
done
mkf "$D/podcast-ep-14.mp3" 48000000 202605151400
mkf "$D/spreadsheet.csv" 9000 202604021100
mkf "$D/tungstate-0.1.0.dmg" 88000000 202608091500
mkf "$D/archive.zip" 12000000 202603211700
mkf "$D/Screenshot 2026-09-01 at 11.02.44.png" 310000 202609011102
# The awkward ones, each reachable on exactly one screen:
mkf "$D/.DS_Store" 6148 202609011102        # ignored by every layout
mkf "$D/no-extension" 1200 202608301200     # falls to the inbox
mkf "$D/a name with spaces.txt" 800 202608301201
mkf "$D/emoji 🎉 name.txt" 800 202608301202
mkdir -p "$D/archive.zip.d" && mkf "$D/archive.zip.d/inner.txt" 100 202608301203
ln -s "$D/readme.pdf" "$D/shortcut.pdf"      # a symlink, which is not followed
mkdir -p "$D/Images" && mkf "$D/Images/already-filed.jpg" 500000 202607041200
touch "$D/written-just-now.txt"              # inside its cooldown, so "too recent"
printf 'fresh\n' >"$D/written-just-now.txt"

# --- 3. photos with nothing but dates ---------------------------------------

P="$DEMO/photos-by-nothing"
i=0
for year in 2024 2025 2026; do
	for n in $(seq 1 40); do
		i=$((i + 1))
		mon=$(printf '%02d' $(((n % 12) + 1)))
		day=$(printf '%02d' $(((n % 27) + 1)))
		mkf "$P/DSC$(printf '%05d' $i).jpg" $((1800000 + i * 300)) "${year}${mon}${day}1000"
	done
done

# --- 4. the conditions that are about the rules, not the files --------------

mkdir -p "$DEMO/empty"

H="$DEMO/huge"
mkdir -p "$H"
n=0
while [ $n -lt 5000 ]; do
	n=$((n + 1))
	d="$H/batch-$((n / 500))"
	mkdir -p "$d"
	: >"$d/file-$(printf '%05d' $n).txt"
done

B="$DEMO/broken-rules"
mkdir -p "$B/.tungstate"
mkf "$B/one.txt" 400 202609011200
cat >"$B/.tungstate/policy.toml" <<'TOML'
# Deliberately invalid: the window has to show the line and column, not a shrug.
[folder]
name = "broken"

[[rule]]
name = "images"
path = "Images
match = { mime = "image/*" }
TOML

U="$DEMO/unsettled"
mkdir -p "$U/.tungstate"
for name in a b c; do mkf "$U/$name.txt" 500 202609011200; done
# `copy-{name}` renders differently once the file is renamed, so every pass
# renames it again. This is the one refusal the window keeps.
cat >"$U/.tungstate/policy.toml" <<'TOML'
# Rules that never settle. Kept as a demo of the refusal, not as an example.
[folder]
name = "unsettled"

[defaults]
cooldown = "0s"

[[rule]]
name = "prefix"
path = ""
rename = "copy-{name}"
match = { ext = "txt" }
TOML

DENIED="$DEMO/denied"
mkdir -p "$DENIED/locked"
mkf "$DENIED/readable.txt" 300 202609011200
mkf "$DENIED/locked/hidden.txt" 300 202609011200
chmod 000 "$DENIED/locked"

# --- 5. the NAS end of the drain half ---------------------------------------

NAS="$DEMO/nas"
mkdir -p "$NAS/incoming"
# (a) same name, different size: the clash dialog.
mkf "$NAS/incoming/talk.mp4" 120000000 202607011130
# (b) byte-for-byte identical to a source file: the "already there" dialog.
cp "$D/holiday.jpg" "$NAS/incoming/holiday.jpg"
touch -t 202606010930 "$NAS/incoming/holiday.jpg"

# Real bytes, because verification reads both copies back and a hole verifies
# instantly. Three files is enough to photograph a run in progress.
mkdir -p "$DEMO/to-drain"
for n in 1 2 3; do
	dd if=/dev/urandom of="$DEMO/to-drain/clip-$n.mov" bs=1048576 count=200 2>/dev/null
	magic mov | dd of="$DEMO/to-drain/clip-$n.mov" conv=notrunc 2>/dev/null
	touch -t "20260${n}101200" "$DEMO/to-drain/clip-$n.mov"
done

# --- 6. the two roots the engine has to build, because guessing them is how
#        a fixture goes subtly wrong ----------------------------------------

CLI="$REPO/target/debug/tungstate"
if [ ! -x "$CLI" ]; then
	echo "building the cli, once, to make the tidy folders with the real engine"
	(cd "$REPO" && cargo build -q -p tungstate-cli)
fi

export TUNGSTATE_JOURNAL="$DEMO/build-journal.db"
export TUNGSTATE_SECRETS=memory

# A folder that is already tidy: make it messy, then let `apply` tidy it. The
# result matches `downloads.toml` by construction rather than by hand.
T="$DEMO/already-tidy"
mkdir -p "$T"
i=0
for name in report summary notes; do
	i=$((i + 1))
	mkf "$T/$name.pdf" $((30000 + i * 700)) "20260${i}121030"
done
for name in one two; do
	i=$((i + 1))
	mkf "$T/$name.jpg" $((900000 + i * 400)) "20260${i}151100"
done
"$CLI" init "$T" --template downloads >/dev/null
"$CLI" apply "$T" --yes >/dev/null

# A folder with rules already, so `give_rules` refuses and the comparison
# screen has a "(the rules you have)" row to sit above the alternatives.
R="$DEMO/has-rules"
mkdir -p "$R"
messy_media "$R"
"$CLI" init "$R" --template media >/dev/null

# The shape-without-rules folder: let the engine file it, then take the rules
# away. What is left is a folder in a real five-level shape that nothing is
# governing, which is exactly what `learn_folder` is for.
"$CLI" init "$DEMO/real-shape" --template media >/dev/null
"$CLI" apply "$DEMO/real-shape" --yes >/dev/null
rm -rf "$DEMO/real-shape/.tungstate"

rm -f "$TUNGSTATE_JOURNAL"
unset TUNGSTATE_JOURNAL TUNGSTATE_SECRETS

mkdir -p "$DEMO/home"

# --- how to run against it --------------------------------------------------

cat <<EOF

built $DEMO

  real-shape/        five levels deep, no rules. What learn_folder is for
  messy-downloads/   flat, with the symlink, the emoji, the too-recent file
  photos-by-nothing/ 120 photos across three years
  already-tidy/      filed by the engine, so tidy means tidy
  has-rules/         a policy already, so give_rules refuses
  broken-rules/      rules that will not parse
  unsettled/         rules that never settle
  huge/              5000 files, for the tree and the blocking tidy
  empty/             nothing at all
  denied/            a directory that cannot be read
  to-drain/, nas/    the two ends of a transfer

Run the window against it, and nothing else:

  cd $REPO/crates/tungstate-gui
  TUNGSTATE_JOURNAL=$DEMO/journal.db \\
  TUNGSTATE_SECRETS=memory \\
  HOME=$DEMO/home \\
    ./ui/node_modules/.bin/tauri dev

Two states this script will not fake, because faking them means writing
journal rows and reaching around the engine:

  an interrupted run   start a transfer of to-drain/, kill -9 the process
                       part-way, reopen the window
  a quarantine         run a link with on_conflict = "quarantine" against
                       nas/incoming, which already holds a different talk.mp4
EOF
