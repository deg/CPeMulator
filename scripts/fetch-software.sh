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

# Extract every file flat into software/ (apps like WordStar/dBASE/Multiplan need
# their full set of overlays, message files, and terminal-config files alongside
# the .COM; doc files are harmless extra). Skip only bulky manuals.
fetch() {
  url="$1"
  zip="$ZIPS/$(basename "$url")"
  printf '==> %-28s ' "$(basename "$url")"
  # Reuse a previously-downloaded zip (re-running is fast and avoids the flaky
  # archive); otherwise download it.
  if [ -s "$zip" ] || curl -fsS --connect-timeout 10 --max-time 90 -o "$zip" "$url" 2>/dev/null; then
    unzip -o -j -C -d "$DEST" "$zip" -x '*.pdf' '*.PDF' >/dev/null 2>&1
    echo "ok"
  else
    echo "SKIPPED (download failed/timed out)"
  fi
}

# --- operating system + utilities ---
fetch "$base/os/STDCPM22.ZIP"       # STAT, PIP, ED, DUMP, SUBMIT, ASM, LOAD, DDT...

# --- languages ---
fetch "$base/lang/mbasic.zip"       # Microsoft MBASIC 5.29
fetch "$base/lang/TP_301A.ZIP"      # Turbo Pascal 3.01a

# --- games ---
fetch "$base/games/zork123_80.zip"  # Zork I, II, III (+ data files)
fetch "$base/misc/LADDER.ZIP"       # Ladder
fetch "$base/games/sargon.zip"      # Sargon chess
fetch "$base/games/nemesis.zip"     # Nemesis (dungeon adventure)
fetch "$base/games/manygame.zip"    # a collection of small games

# --- iconic productivity (full-screen: best with a matching terminal) ---
fetch "$base/text/Ws4_mbee.zip"     # WordStar 4 (Microbee — ADM-3A-compatible)
fetch "$base/business/Suprcalc.zip" # SuperCalc spreadsheet
fetch "$base/business/MULTPLAN.ZIP" # Microsoft Multiplan
fetch "$base/dbase/dbaseii.zip"     # dBASE II database

# A guide to the software, written into the disk as PROGRAMS.TXT (CR/LF line
# endings + a trailing ^Z so CP/M's TYPE displays it cleanly). load-all.sh copies
# it onto drive A:, so inside CP/M you can `TYPE PROGRAMS.TXT`.
awk '{ printf "%s\r\n", $0 }' > "$DEST/PROGRAMS.TXT" <<'GUIDE'
================================================================
            CP/M 2.2  -  PROGRAMS ON THIS DISK
================================================================

Run a program by typing its name at the A> prompt.  BASIC games
(names ending in .BAS) are run with:   MBASIC NAME.BAS

Leaving a program:
  - Most have their own quit command (noted below).
  - Ctrl-C returns to A> when the program cooperates.
  - In MBASIC, type  SYSTEM
  - To quit the emulator entirely, press  Ctrl-\

Tip: some old programs only accept CAPITAL letters.  Start cpemu
with  --upper , or just hold Shift.

---------------- GAMES & ADVENTURES ----------------------------

ZORK1  ZORK2  ZORK3  Infocom's legendary text adventures.  Try
                     OPEN MAILBOX, READ LEAFLET, GO NORTH, ...
ADVENTUR             Colossal Cave Adventure - the original.
SARGON               Sargon - play a full game of chess.
LADDER               Arcade platformer (try --mhz=2 if too fast).
STAR.BAS             Star Trek.  Run:  MBASIC STAR.BAS
                     (answer the questions with a CAPITAL Y).
... and many more .BAS games - run them with  MBASIC <name>.BAS

---------------- LANGUAGES -------------------------------------

MBASIC               Microsoft BASIC-80.  Write and RUN programs.
                     Exit with the  SYSTEM  command.
TURBO                Borland Turbo Pascal 3.01a - edit, compile,
                     and run Pascal.

---------------- PRODUCTIVITY ----------------------------------

DBASE                dBASE II - the classic database.  Quit with
                     the  QUIT  command.
MP                   Microsoft Multiplan - a spreadsheet.
SC                   SuperCalc spreadsheet (needs INSTALL first).
WS                   WordStar word processor (needs WINSTALL).

---------------- UTILITIES -------------------------------------

DIR                  List the files on a drive (built into CP/M).
STAT                 Disk and file status; free space.
PIP                  Copy files, e.g.  PIP NEW.TXT=OLD.TXT
ED                   A line-oriented text editor.
TYPE FILE            Show a text file (e.g. TYPE PROGRAMS.TXT).
DDT                  The CP/M debugger.   ASM - 8080 assembler.
DUMP FILE            Hex-dump a file.

================================================================
  Built with cpemu, a from-scratch Z80 CP/M emulator in Rust.
================================================================
GUIDE
printf '\032' >> "$DEST/PROGRAMS.TXT"   # ^Z end-of-text marker for CP/M

echo
echo "Programs in $DEST:"
ls -1 "$DEST" | grep -viE '^_zips$' | sort | column 2>/dev/null || ls -1 "$DEST" | grep -viE '^_zips$' | sort
echo
echo "Boot with everything:  scripts/load-all.sh"
