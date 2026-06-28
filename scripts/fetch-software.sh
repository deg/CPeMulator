#!/bin/sh
# Fetch classic CP/M software into ./software/ for use with cpemu
# (e.g. `cargo run -- software/MBASIC.COM`, or `scripts/load-all.sh`).
#
# These are commercial-heritage binaries preserved at retroarchive.org. They are
# NOT committed to this repository (software/ is gitignored); this script only
# downloads them locally. Downloads use timeouts and tolerate failures (the
# archive is occasionally slow), so a stalled item is skipped, not fatal.

DEST="$(cd "$(dirname "$0")/.." && pwd)/software"
ZIPS="$DEST/_zips"
mkdir -p "$ZIPS"
base="http://www.retroarchive.org/cpm"

EXTRA_DIR="$DEST/extras"

# Start from a clean set of extracted files (keep the cached zips) so re-fetching
# doesn't leave stale files around.
find "$DEST" -maxdepth 1 -type f -delete 2>/dev/null || true
rm -rf "$EXTRA_DIR"
mkdir -p "$EXTRA_DIR"

# We split each zip into two places:
#   software/         programs + their support files (loaded onto the CP/M disk)
#   software/extras/  everything else — scanned manuals (images/PDFs), source,
#                     and build artifacts (kept so they're discoverable, but NOT
#                     loaded; otherwise a few zips full of manual scans would
#                     fill the whole 8 MB disk).
# EXTRAS lists what counts as "not a program". -C makes patterns case-insensitive.
EXTRAS="*.pdf *.jpg *.jpeg *.png *.gif *.bmp *.tif *.tiff \
        *.asm *.mac *.asc *.obj *.rel *.sym *.lst *.prn *.bak *.zip *.ark *.lbr \
        *.lsp *.pas *.for *.pli *.inc \
        *.txt *.doc *.me *.1st"
fetch() {
  url="$1"
  zip="$ZIPS/$(basename "$url")"
  printf '==> %-28s ' "$(basename "$url")"
  # Reuse a previously-downloaded zip (re-running is fast and avoids the flaky
  # archive); otherwise download it.
  if [ -s "$zip" ] || curl -fsS --connect-timeout 10 --max-time 90 -o "$zip" "$url" 2>/dev/null; then
    # shellcheck disable=SC2086
    unzip -o -j -C -d "$DEST" "$zip" -x $EXTRAS >/dev/null 2>&1       # programs
    # shellcheck disable=SC2086
    unzip -o -j -C -d "$EXTRA_DIR" "$zip" $EXTRAS >/dev/null 2>&1     # the rest
    echo "ok"
  else
    echo "SKIPPED (download failed/timed out)"
  fi
}

# --- operating system + utilities ---
fetch "$base/os/STDCPM22.ZIP"       # STAT, PIP, ED, DUMP, SUBMIT, ASM, LOAD, DDT...

# --- languages & compilers ---
fetch "$base/lang/mbasic.zip"       # Microsoft MBASIC 5.29 (interpreter)
fetch "$base/lang/TP_301A.ZIP"      # Turbo Pascal 3.01a
fetch "$base/lang/lisp80.zip"       # LISP/80 + muLISP (Software Warehouse) — CP/M Lisp
fetch "$base/lang/MSFORTRAN343.ZIP" # Microsoft FORTRAN-80 v3.43 (F80 + L80)
fetch "$base/lang/CB80.ZIP"         # CBASIC — DRI's CB-80 BASIC compiler
# (BDS C is fetched separately below — it lives at bdsoft.com, not retroarchive.)

# --- games ---
fetch "$base/games/zork123_80.zip"  # Zork I, II, III (+ data files)
fetch "$base/misc/LADDER.ZIP"       # Ladder
fetch "$base/misc/CATCHUM.ZIP"      # Catchum — a Pac-Man clone
fetch "$base/games/sargon.zip"      # Sargon chess
fetch "$base/games/nemesis.zip"     # Nemesis (dungeon adventure)
fetch "$base/games/manygame.zip"    # a collection of small games

# --- iconic productivity (full-screen: best with a matching terminal) ---
fetch "$base/text/Ws4_mbee.zip"     # WordStar 4 (Microbee — ADM-3A-compatible)
fetch "$base/business/Suprcalc.zip" # SuperCalc spreadsheet
fetch "$base/business/MULTPLAN.ZIP" # Microsoft Multiplan
fetch "$base/dbase/dbaseii.zip"     # dBASE II database

# BDS C — the classic public-domain CP/M C compiler. It's special-cased because
# (a) it lives at the author's site (bdsoft.com), not retroarchive, and (b) the
# archive bundles a Z80 variant, full library source, and docs. We load only the
# canonical 8080 build (the bdsc160/ subdirectory), and within it skip the *.c /
# *.csm library source — those aren't needed to compile programs (the runtime is
# precompiled in the .CRL libraries). CC compiles and CLINK links straight to a
# .COM with no separate assembler, so a few files give a complete C toolchain.
# Everything outside bdsc160/ goes to extras/.
bdsc="$ZIPS/bdsc-all.zip"
printf '==> %-28s ' "bdsc-all.zip (BDS C)"
if [ -s "$bdsc" ] || curl -fsSL --connect-timeout 15 --max-time 120 -o "$bdsc" "https://www.bdsoft.com/dist/bdsc-all.zip" 2>/dev/null; then
  # shellcheck disable=SC2086
  unzip -o -j -C -d "$DEST" "$bdsc" "bdsc160/*" -x $EXTRAS "*.c" "*.csm" >/dev/null 2>&1
  unzip -o -j -C -d "$EXTRA_DIR" "$bdsc" -x "bdsc160/*" >/dev/null 2>&1
  echo "ok"
else
  echo "SKIPPED (download failed/timed out)"
fi

# Second-pass trim: a few archives bundle, alongside the actual program,
# terminal-definition overlays (SuperCalc's per-terminal patch files) and CP/M
# system-regeneration tools (MOVCPM/SYSGEN/...). Those are install fragments and
# system-build utilities, not things you run at the A> prompt, and ~30 of them
# badly clutter DIR. The extension-based split above can't tell them apart from
# real programs (they're all .COM), so move this explicit list to extras/ — they
# stay discoverable under software/extras/, just off the working disk. (-iname
# makes the match case-insensitive, as the archives mix cases.)
NOISE="
  2ADM3.COM 2DEFAULT.COM 2SUPERBR.COM 2VECTOR.COM 2XEROX.COM 2ZENITH.COM
  3ADM3.COM 3DEFAULT.COM 3SUPERBR.COM 3VECTOR.COM 3XEROX.COM 3ZENITH.COM
  EADM3.COM EDEFAULT.COM ESUPERBR.COM EVECTOR.COM EXEROX.COM EZENITH.COM
  SCHAZ15.COM
  INSTALL.COM INSTALL.DAT INSTALL.LD INSTALL.MSG INSTALL.OVL INSTALL.SPC
  MOVCPM.COM SYSGEN.COM CCSYSGEN.COM CCSINIT.COM CPM24CCS.COM RLOCBIOS.COM
  GENMOD.COM MOVEPRN.COM FINDBAD.COM BACKUP.COM
  -CCSCPM.251 ZOSO.22 CATALOG.22 CATALOG.24 CATALOG.26 DISKDEF.LIB
"
for n in $NOISE; do
  find "$DEST" -maxdepth 1 -iname "$n" -exec mv -f {} "$EXTRA_DIR/" \; 2>/dev/null || true
done

# Write the committed guide (scripts/programs.txt) onto the disk as PROGRAMS.TXT,
# converting to CR/LF and adding a trailing ^Z so CP/M's TYPE displays it cleanly.
# (The guide's source is version-controlled; software/PROGRAMS.TXT is generated.)
guide="$(cd "$(dirname "$0")" && pwd)/programs.txt"
if [ -f "$guide" ]; then
  awk '{ printf "%s\r\n", $0 }' "$guide" > "$DEST/PROGRAMS.TXT"
  printf '\032' >> "$DEST/PROGRAMS.TXT"   # ^Z end-of-text marker for CP/M
fi

prog_kb=$(ls -l "$DEST" | awk 'NR>1 && $1 !~ /^d/ {s+=$5} END {print int(s/1024)}')
extra_n=$(find "$EXTRA_DIR" -maxdepth 1 -type f 2>/dev/null | wc -l | tr -d ' ')
echo
echo "Programs in $DEST  (~${prog_kb} KB, loaded onto the CP/M disk):"
ls -1 "$DEST" | grep -viE '^_zips$|^extras$' | sort | column 2>/dev/null || ls -1 "$DEST" | grep -viE '^_zips$|^extras$' | sort
echo
echo "Manuals/source/docs kept in $EXTRA_DIR ($extra_n files, not loaded)."
echo "Boot with everything:  scripts/load-all.sh"
