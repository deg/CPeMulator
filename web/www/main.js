// CPeMulator web page: boots the WebAssembly emulator into an xterm.js terminal
// and wires up the control panel. See dev-docs/browser.md.
//
// The emulator never blocks (see crates/cpemu/src/session.rs). Each animation
// frame runs a frame's worth of Z80 cycles at the chosen clock; when CP/M is
// waiting for a key, the loop stops until a key arrives, so an idle page costs
// nothing.

import { Terminal } from "./vendor/xterm/xterm.mjs";
import init, { Emulator } from "./pkg/cpemu_web.js";

const $ = (id) => document.getElementById(id);
const encoder = new TextEncoder();
const MAX_FRAME_MS = 100; // after a stall or a hidden tab, don't sprint to catch up
const MAX_SLICE_MS = 12; // "Max" speed: run this long per frame, leaving time to draw
const ENDS_PROGRAM =
  "This restarts CP/M, which ends the program that is running. Anything it hasn't saved will be lost.";

// ---------------------------------------------------------------------------
// Status line

let statusNote = ""; // a message about the last action, shown before the state
let statusError = false;
function note(text, isError = false) {
  statusNote = text;
  statusError = isError;
  showStatus();
}

// ---------------------------------------------------------------------------
// Saving drive A: in this browser (IndexedDB). Every call tolerates storage
// being unavailable (a private window, blocked site data): the page still
// works, it just forgets the disk when closed.

let db = null;
function openDb() {
  return new Promise((resolve) => {
    try {
      const req = indexedDB.open("cpemulator", 1);
      req.onupgradeneeded = () => req.result.createObjectStore("disks");
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => resolve(null);
      req.onblocked = () => resolve(null);
    } catch {
      resolve(null);
    }
  });
}
function store(mode, fn) {
  return new Promise((resolve, reject) => {
    const tx = db.transaction("disks", mode);
    const req = fn(tx.objectStore("disks"));
    tx.oncomplete = () => resolve(req.result);
    tx.onerror = tx.onabort = () => reject(tx.error);
  });
}
async function loadSavedDisk() {
  if (!db) return null;
  try {
    return (await store("readonly", (s) => s.get("A"))) ?? null;
  } catch {
    return null;
  }
}

// Browsers treat this storage as disposable. Safari deletes a site's data after
// about seven days of use without a visit; any browser may clear it when space
// runs low, when history is cleared, or at the end of a private window. So the
// browser copy is a convenience, and a saved .dsk (Save disk) is the user's
// real copy. The page tracks whether the disk has changed since it was last
// saved ("backed up"), keeps that flag in localStorage, and says so in the
// status line with a Save disk link. (localStorage, not IndexedDB, because it
// writes synchronously: an IndexedDB write started by the Save disk click is
// lost if the tab closes or reloads a moment later. Either store being cleared
// alone errs safe: no flag reads as "not backed up", and no image boots a fresh
// disk.) It also asks the browser to keep the data (persist()), which the
// browser may refuse.
//
// Words: the copy in this browser is "kept"; a .dsk file on the visitor's
// computer is "saved".
let backedUp = true; // has drive A: been saved since it last changed?
let baseline = -1; // emu.diskChanges() when it last matched the backed-up disk
let persisted = false;
function storeBackedUp() {
  try {
    localStorage.setItem("cpemulator.A.backedUp", String(backedUp));
  } catch {
    // The flag is only advice; the image is what matters.
  }
}
function loadBackedUp() {
  try {
    return localStorage.getItem("cpemulator.A.backedUp") === "true";
  } catch {
    return false;
  }
}
function setBackedUp(value) {
  baseline = emu.diskChanges();
  if (backedUp === value) return;
  backedUp = value;
  storeBackedUp();
  showStatus();
  renderOpenPanel();
}
// Called after every slice: the first change to a backed-up disk clears the
// flag, and is also the moment there is something worth asking the browser to
// keep.
function noteDiskChange() {
  if (!backedUp || emu.diskChanges() === baseline) return;
  backedUp = false;
  storeBackedUp();
  renderOpenPanel();
  if (!persisted && navigator.storage?.persist) {
    navigator.storage.persist().then((ok) => {
      persisted = ok;
      showStatus();
    }, () => {});
  }
}

let savedChanges = -1; // emu.diskChanges() when the disk was last saved
let saving = false;
let lastSave = 0;
let saveState = "";
async function saveDisk() {
  if (!db || saving) return;
  saving = true;
  const changes = emu.diskChanges();
  const image = emu.diskImage();
  try {
    await store("readwrite", (s) => s.put(image, "A"));
    savedChanges = changes;
    saveState = "disk kept in this browser";
  } catch (e) {
    saveState = `disk NOT kept in this browser (${e?.name ?? e})`;
  }
  saving = false;
  lastSave = performance.now();
  showStatus();
}
// Save when the disk changed: at most once a second while a program runs, and
// at once when CP/M goes idle or the page is being left.
function maybeSave(force) {
  if (emu.diskChanges() === savedChanges) return;
  if (force || performance.now() - lastSave > 1000) saveDisk();
}
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "hidden") maybeSave(true);
});

// The software library: the desktop's list as links (library.json). If it
// can't be loaded, the page works without it.
async function loadLibrary() {
  try {
    const r = await fetch("library.json");
    if (r.ok) return await r.json();
  } catch {
    // Fall through.
  }
  return { exclude: [], items: [] };
}

// The library entry a dropped archive is, if any. First by its file name as
// its link downloads it (ignoring case and the " (1)" a browser adds to a
// second copy); failing that, by what it holds: the entry's identifying
// program, and its folders if it takes only some. So a renamed download, or a
// copy from another site, still gets its rules. Only an unambiguous match
// counts; any other archive is unpacked whole.
function libraryEntry(fileName, bytes) {
  const name = fileName.toLowerCase().replace(/ \(\d+\)(?=\.zip$)/, "");
  const byName = library.items.find(
    (item) => decodeURIComponent(item.url.split("/").pop()).toLowerCase() === name,
  );
  if (byName) return byName;
  let paths;
  try {
    paths = Emulator.zipPaths(bytes).map((p) => p.toLowerCase());
  } catch {
    return undefined;
  }
  const names = new Set(paths.map((p) => p.split("/").pop()));
  const matches = library.items.filter(
    (item) =>
      names.has(item.check.toLowerCase()) &&
      (!item.only || item.only.some((o) => paths.some((p) => p.includes(o.toLowerCase())))),
  );
  return matches.length === 1 ? matches[0] : undefined;
}


function loadSpeed() {
  try {
    return localStorage.getItem("cpemulator.speed") ?? "4";
  } catch {
    return "4";
  }
}
function storeSpeed(value) {
  try {
    localStorage.setItem("cpemulator.speed", value);
  } catch {
    // Not remembered; harmless.
  }
}

// ---------------------------------------------------------------------------
// Boot

const term = new Terminal({
  cols: 80,
  rows: 24,
  cursorBlink: true,
  fontFamily: 'ui-monospace, Menlo, Consolas, "DejaVu Sans Mono", monospace',
  fontSize: 15,
});
term.open($("term"));

await init();
db = await openDb();
const library = await loadLibrary();
// Which library archives have been started downloading on this page, so
// their rows say what to do next. Kept in memory only: the page can't see
// whether a download arrived, or whether the file is still in the visitor's
// Downloads folder later, so after a reload a remembered "started" would be
// stale. ("On disk" comes from the disk itself, and does persist.)
const downloaded = new Set();
saveState = db ? "disk kept in this browser" : "disk not kept (this browser's storage is unavailable)";

// Declared before anything can call showStatus() or renderOpenPanel(), which
// read them.
let state = "running"; // the last emu.run() result
let openPanel = null; // "software", "disk", or null
let speed = loadSpeed();
const speedSelect = $("speed");
if ([...speedSelect.options].some((o) => o.value === speed)) speedSelect.value = speed;
else speed = speedSelect.value;

let emu;
const savedDisk = await loadSavedDisk();
try {
  persisted = (await navigator.storage?.persisted?.()) ?? false;
} catch {
  persisted = false;
}
try {
  emu = savedDisk ? Emulator.fromImage(savedDisk) : new Emulator();
  if (savedDisk) {
    savedChanges = emu.diskChanges();
    // A disk saved before the flag existed counts as not backed up.
    backedUp = loadBackedUp();
    note("Welcome back: drive A: is as you left it.");
  }
} catch (e) {
  emu = new Emulator();
  note(`Your saved disk could not be used (${e.message ?? e}), so this is a fresh one.`, true);
}

baseline = emu.diskChanges();

// ---------------------------------------------------------------------------
// The run loop

let scheduled = false;
let last = 0;

// A library entry's setup, run after its files go on the disk: keys typed one
// at a time, each only when CP/M is waiting for a key. (Installers like
// WordStar's WINSTALL throw away keys typed ahead.) Then a check that it
// worked. The visitor's own typing is held off meanwhile.
let setup = null; // { keys: number[], entry }
function runSetup() {
  if (!setup || state !== "idle") return;
  if (setup.keys.length) {
    emu.typeBytes(new Uint8Array([setup.keys.shift()]));
    wake();
    return;
  }
  const { entry } = setup;
  setup = null;
  const bytes = emu.readFile(entry.setup.file);
  const ok = bytes && new TextDecoder("latin1").decode(bytes).includes(entry.setup.contains);
  note(
    ok ? `${entry.title} is ready.` : `${entry.title}: ${entry.setup.what} didn't work.`,
    !ok,
  );
}

function wake() {
  if (scheduled || !(state === "running" || state === "idle")) return;
  scheduled = true;
  last = performance.now();
  requestAnimationFrame(frame);
}

function frame(now) {
  scheduled = false;
  const dt = Math.min(now - last, MAX_FRAME_MS);
  last = now;
  if (speed === "max") {
    const end = performance.now() + MAX_SLICE_MS;
    do state = emu.run(200_000);
    while (state === "running" && performance.now() < end);
  } else {
    state = emu.run(Math.max(1, Math.round(Number(speed) * 1000 * dt)));
  }
  const out = emu.takeOutput();
  if (out.length) term.write(out);
  noteDiskChange();
  if (state === "running") {
    scheduled = true;
    requestAnimationFrame(frame);
    maybeSave(false);
  } else {
    maybeSave(true);
    renderOpenPanel();
    renderWelcome();
    runSetup();
  }
  showStatus();
}

function showStatus() {
  const clock = speed === "max" ? "full speed" : `${speed} MHz`;
  const what = {
    running: "running",
    idle: "waiting for a key",
  }[state] ?? `stopped (${state}): use Restart CP/M`;
  const backup = backedUp ? "" : "changed since you last saved it:";
  const parts = [statusNote, `${clock} Z80 · ${what}`, saveState, backup].filter(Boolean);
  $("status-text").textContent = parts.join(" · ");
  $("save-nudge").hidden = backedUp;
  $("status").classList.toggle("error", statusError || !(state === "running" || state === "idle"));
}

// Replace the running machine (reset, a new disk) and start it. `fresh` says
// whether the new disk is one the user can't lose (the bundled disk, or a .dsk
// they uploaded and so already have): it counts as backed up.
function swapEmulator(next, fresh) {
  emu.free();
  emu = next;
  if (fresh) setBackedUp(true);
  else baseline = emu.diskChanges(); // same contents, new counter
  savedChanges = -1; // save the new machine's disk at the first chance
  setup = null; // a half-run install belonged to the old machine
  term.reset();
  state = "running";
  wake();
  renderOpenPanel();
}

// ---------------------------------------------------------------------------
// Keys

let ctrlArmed = false;
const ctrlButton = $("k-ctrl");
function setCtrl(on) {
  ctrlArmed = on;
  ctrlButton.setAttribute("aria-pressed", String(on));
}

function send(bytes) {
  emu.typeBytes(bytes);
  // Typing means the last message has been seen.
  statusNote = "";
  statusError = false;
  wake();
}

term.onData((text) => {
  if (setup) {
    note(`Wait a moment: ${setup.entry.setup.what}.`);
    return;
  }
  if (ctrlArmed) {
    setCtrl(false);
    const c = text.charCodeAt(0);
    // Ctrl + a letter or one of @ [ \ ] ^ _ is that character's low five bits.
    if (text.length === 1 && c >= 0x40 && c < 0x80) {
      send(new Uint8Array([c & 0x1f]));
      return;
    }
  }
  send(encoder.encode(text));
});

// Buttons give focus back to the terminal, so typing continues there.
function onClick(id, fn) {
  $(id).addEventListener("click", (e) => {
    fn(e);
    term.focus();
  });
}
onClick("k-esc", () => send(new Uint8Array([0x1b])));
// Control keys as buttons: Ctrl-C, and the three that browsers on Windows and
// Linux keep for themselves (close tab, new tab, new window), which WordStar
// and Turbo Pascal need.
for (const [id, code] of [["k-ctrlc", 0x03], ["k-ctrlw", 0x17], ["k-ctrlt", 0x14], ["k-ctrln", 0x0e]]) {
  onClick(id, () => send(new Uint8Array([code])));
}
onClick("k-ctrl", () => setCtrl(!ctrlArmed));

// ---------------------------------------------------------------------------
// Disk: upload, download, the file list

function download(bytes, name) {
  const url = URL.createObjectURL(new Blob([bytes], { type: "application/octet-stream" }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
}

async function putFiles(fileList) {
  const files = [...fileList];
  if (!files.length) return;
  const images = files.filter((f) => /\.dsk$/i.test(f.name));
  if (images.length) {
    if (files.length > 1) {
      note("Load a .dsk disk image on its own: it replaces the whole disk.", true);
      return;
    }
    const f = images[0];
    if (!confirm(`Replace drive A: with ${f.name}? Its current files are lost unless you save the disk first.`)) return;
    try {
      swapEmulator(Emulator.fromImage(new Uint8Array(await f.arrayBuffer())), true);
      note(`Drive A: is now ${f.name}.`);
    } catch (e) {
      note(`${f.name}: ${e.message ?? e}`, true);
    }
    return;
  }
  // Putting files on the disk warm-boots CP/M (so it sees them), which ends a
  // running program. At the A> prompt nothing is lost; otherwise ask.
  if (emu.inProgram() && !confirm(ENDS_PROGRAM + " Add the files anyway?")) return;
  const done = [];
  const failed = [];
  const recognized = []; // library entries among the files
  for (const f of files) {
    try {
      const bytes = new Uint8Array(await f.arrayBuffer());
      if (/\.zip$/i.test(f.name)) {
        // Most CP/M software comes zipped: unpack it onto the disk. An archive
        // from the library gets the desktop's rules for it; any other, all of
        // its files.
        const entry = libraryEntry(f.name, bytes);
        const only = entry?.only ?? [];
        const exclude = entry ? [...library.exclude, ...(entry.exclude ?? [])] : [];
        const report = emu.putZip(bytes, only, exclude);
        done.push(...report.put);
        failed.push(...report.skipped.map((why) => `${f.name}, ${why}`));
        report.free();
        if (entry) recognized.push(entry);
      } else {
        emu.putFile(f.name, bytes);
        done.push(f.name.toUpperCase());
      }
    } catch (e) {
      failed.push(`${f.name}: ${e.message ?? e}`);
    }
  }
  if (done.length) {
    // putFile warm-boots CP/M so it sees the new files; show its fresh prompt.
    state = "running";
    wake();
    renderOpenPanel();
  }
  const withSetup = recognized.find((e) => e.setup);
  if (withSetup) {
    setup = { keys: [...encoder.encode(withSetup.setup.keys)], entry: withSetup };
  }
  // A big archive can hold dozens of files: name a few, count the rest.
  const list = (names) =>
    names.length <= 8 ? names.join(", ") : `${names.slice(0, 6).join(", ")} and ${names.length - 6} more`;
  const msg = [
    recognized.length ? `From the library: ${recognized.map((e) => e.title).join(", ")}.` : "",
    done.length ? `Put on A: ${list(done)}.` : "",
    failed.length ? `Not put on A: ${list(failed)}.` : "",
  ]
    .filter(Boolean)
    .join(" ");
  note(withSetup ? `${msg} Now ${withSetup.setup.what}…` : msg, failed.length > 0);
}

// Add files (programs, data, .zip archives) and Load disk (a .dsk) each have
// their own file picker; both go through putFiles, which tells them apart.
for (const [button, input] of [["add-files", "add-input"], ["load-disk", "load-input"]]) {
  onClick(button, () => $(input).click());
  $(input).addEventListener("change", () => {
    putFiles($(input).files);
    $(input).value = "";
  });
}

// Files can be dropped anywhere on the page. While they're dragged over it, an
// overlay covers the whole page, so it's clear the drop target is everywhere,
// not some small zone. (dragenter and dragleave fire for every element
// crossed, hence the depth count.)
const overlay = $("drop-overlay");
let dragDepth = 0;
const draggingFiles = (e) => [...(e.dataTransfer?.types ?? [])].includes("Files");
document.addEventListener("dragenter", (e) => {
  if (!draggingFiles(e)) return;
  e.preventDefault();
  dragDepth += 1;
  overlay.hidden = false;
});
document.addEventListener("dragover", (e) => {
  if (!draggingFiles(e)) return;
  e.preventDefault();
  e.dataTransfer.dropEffect = "copy";
});
document.addEventListener("dragleave", (e) => {
  if (!draggingFiles(e)) return;
  dragDepth = Math.max(0, dragDepth - 1);
  if (dragDepth === 0) overlay.hidden = true;
});
document.addEventListener("drop", (e) => {
  if (!draggingFiles(e)) return;
  e.preventDefault();
  dragDepth = 0;
  overlay.hidden = true;
  putFiles(e.dataTransfer.files);
});

// Save disk: drive A: as a .dsk file on the visitor's computer, their real copy.
// From the Your disk panel, the welcome card, and the status line's link.
function saveCopy() {
  download(emu.diskImage(), "cpemulator-A.dsk");
  setBackedUp(true);
  note("Saved drive A: as cpemulator-A.dsk, in your Downloads. Load disk brings it back.");
}
onClick("save-disk", saveCopy);
onClick("save-nudge", saveCopy);
onClick("welcome-save", saveCopy);

// The Your disk panel: a summary, whether there are unsaved changes, and the
// files (each saves a copy of itself when clicked).
function renderFiles() {
  const names = emu.files().sort();
  $("disk-summary").textContent = `Drive A: · ${names.length} file${names.length === 1 ? "" : "s"}`;
  $("disk-backup").textContent = backedUp
    ? "No changes since you last saved it (or since it was fresh)."
    : "Changed since you last saved it. Save disk keeps your work on your computer.";
  const list = $("files-list");
  list.replaceChildren(
    ...names.map((name) => {
      const li = document.createElement("li");
      const b = document.createElement("button");
      b.textContent = name;
      b.title = `Save a copy of ${name}`;
      b.addEventListener("click", () => {
        const bytes = emu.readFile(name);
        if (bytes) download(bytes, name);
      });
      li.append(b);
      return li;
    }),
  );
}
// ---------------------------------------------------------------------------
// Panels: one open at a time, beside the terminal (or over the page on a
// narrow screen). Its tab shows it as selected.

const PANELS = { software: "tab-software", disk: "tab-disk" };
function showPanel(name) {
  openPanel = name;
  for (const [panel, tab] of Object.entries(PANELS)) {
    $(panel).hidden = panel !== name;
    $(tab).setAttribute("aria-expanded", String(panel === name));
  }
  renderOpenPanel();
}
function renderOpenPanel() {
  if (openPanel === "software") renderLibrary();
  if (openPanel === "disk") renderFiles();
}
for (const [panel, tab] of Object.entries(PANELS)) {
  onClick(tab, () => showPanel(openPanel === panel ? null : panel));
}
for (const close of document.querySelectorAll("[data-close]")) {
  close.addEventListener("click", () => {
    showPanel(null);
    term.focus();
  });
}

// The welcome card: on a first visit, until dismissed or until some library
// software is on the disk.
function welcomeDismissed() {
  try {
    return localStorage.getItem("cpemulator.welcome.dismissed") === "true";
  } catch {
    return false;
  }
}
function renderWelcome() {
  const onDisk = new Set(emu.files());
  const hasSoftware = library.items.some((i) => onDisk.has(i.check));
  $("welcome").hidden = welcomeDismissed() || hasSoftware;
}
onClick("welcome-close", () => {
  try {
    localStorage.setItem("cpemulator.welcome.dismissed", "true");
  } catch {
    // Shown again next visit; harmless.
  }
  renderWelcome();
});
onClick("welcome-software", () => showPanel("software"));
// The Get software table: a heading row per group, then per program its name
// (which opens a how-to row), what it is, its download size, and a status: Get,
// then "Drop it on this page" (with "get again") once its download has started,
// then "On disk" once its program is on the disk.
const openRows = new Set(); // ids whose how-to is showing

function sizeText(bytes) {
  return bytes >= 1e6 ? `${(bytes / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round(bytes / 1e3))} KB`;
}

function el(tag, props = {}, ...children) {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children);
  return e;
}

// Start one download, from a hidden frame rather than a link. A link click
// navigates the page itself: the archive sites send .zip files as downloads, so
// normally the page stays, but an error page instead of a .zip (a withdrawn
// capture, say) would carry the visitor away from CPeMulator.
//
// The frame also tells us when the site refused. A file sent as a download
// never becomes a page in the frame, so the frame fires no load event; a page
// the site sends instead (a missing capture, a 404, "too many requests") does
// load, and fires one. Tested against the real Wayback Machine in Chrome 159,
// Firefox and WebKit (Safari's engine): a good file, 0 load events; a missing
// capture, a 404 or a 429, 1. So a load event calls `refused`. (Being offline
// fires one only in Chrome; getPrograms' reachability check covers that.)
//
// The HTTPS copy is used when there is one: a secure page's downloads from a
// plain-HTTP site prompt, or are blocked. The frame goes once the download has
// long since started (removing it doesn't cancel a download already handed to
// the browser).
function startDownload(item, refused) {
  const frame = el("iframe", { src: item.mirror ?? item.url, hidden: true, title: "download" });
  frame.addEventListener("load", () => refused(item), { once: true });
  document.body.append(frame);
  setTimeout(() => frame.remove(), 5 * 60_000);
}

const downloadUrl = (item) => item.mirror ?? item.url;
const hostOf = (url) => new URL(url).hostname.replace(/^www\./, "");

// Whether a site answers at all. The page can't read the answer (the archive
// sites send no CORS headers), but a request that can't get through (offline,
// a DNS failure, the site down) rejects. That is the failure the frame's load
// event misses outside Chrome. Returns null if the site answered, else why not,
// as a clause naming the site. A site that answers but refuses the file is
// caught per download, by startDownload's load event.
const PROBE_TIMEOUT_MS = 30_000;
async function unreachable(url) {
  const host = hostOf(url);
  if (navigator.onLine === false) return "your browser is offline";
  const ctl = new AbortController();
  const timer = setTimeout(() => ctl.abort(), PROBE_TIMEOUT_MS);
  try {
    await fetch(url, { method: "HEAD", mode: "no-cors", cache: "no-store", signal: ctl.signal });
    return null;
  } catch (e) {
    return e.name === "AbortError"
      ? `${host} gave no answer in ${PROBE_TIMEOUT_MS / 1000} seconds`
      : `${host} can't be reached (are you online?)`;
  } finally {
    clearTimeout(timer);
  }
}

// Get programs: start their downloads (2 s apart when there are several:
// browsers treat a burst of downloads as suspect and ask once whether to allow
// several, and the Wayback Machine, which serves most of them, limits request
// rates), and meanwhile check each site once. The check runs alongside rather
// than first, since a busy site can take seconds to answer even when all is
// well. Two ways a download is marked as failed, so none fails silently:
//  - the site doesn't answer: all its programs here, with any of their
//    downloads still waiting cancelled;
//  - the site answers with a page instead of the file, which each download's
//    frame reports, however late in a Get all it happens.
// Either way the row says "Didn't download" and the status line lists every
// program that didn't, and why.
const DOWNLOAD_GAP_MS = 2000;
const failed = new Map(); // program id -> why its download couldn't start

// Say what didn't download, grouped by reason, in the status line.
function reportFailures() {
  if (!failed.size) return;
  const byReason = new Map();
  for (const [id, why] of failed) {
    const title = library.items.find((i) => i.id === id)?.title ?? id;
    byReason.set(why, [...(byReason.get(why) ?? []), title]);
  }
  const parts = [...byReason].map(([why, titles]) => `Couldn't download ${titles.join(", ")}: ${why}.`);
  note(`${parts.join(" ")} Try again in a minute; each program's row also lists its original site.`, true);
}

// A download's frame loaded a page instead: the site refused the file.
function refused(item) {
  if (failed.has(item.id) || !downloaded.has(item.id)) return; // already reported
  downloaded.delete(item.id);
  failed.set(
    item.id,
    `${hostOf(downloadUrl(item))} sent a page instead of the file (it may be busy, or the file gone)`,
  );
  reportFailures();
  renderLibrary();
}

function getPrograms(items) {
  const waiting = new Map(); // program id -> timer of a download not yet started
  items.forEach((item, k) => {
    failed.delete(item.id);
    downloaded.add(item.id);
    waiting.set(
      item.id,
      setTimeout(() => {
        waiting.delete(item.id);
        startDownload(item, refused);
      }, k * DOWNLOAD_GAP_MS),
    );
  });
  const hosts = new Map(); // host -> its programs among these
  for (const item of items) {
    const host = hostOf(downloadUrl(item));
    hosts.set(host, [...(hosts.get(host) ?? []), item]);
  }
  for (const hostItems of hosts.values()) {
    unreachable(downloadUrl(hostItems[0])).then((why) => {
      if (!why) return;
      for (const item of hostItems) {
        clearTimeout(waiting.get(item.id));
        downloaded.delete(item.id);
        failed.set(item.id, why); // more specific than a refusal: replaces one
      }
      reportFailures();
      renderLibrary();
    });
  }
  renderLibrary();
}

// Where a program comes from, as links: its secure copy and its original site
// when it has both, so a visitor whose download didn't arrive can try the other.
function sources(item) {
  const link = (url, text) => {
    const a = el("a", { href: url, target: "_blank", rel: "noopener", textContent: text });
    a.addEventListener("click", () => {
      // A new tab shows its own errors, so no check is needed here.
      failed.delete(item.id);
      downloaded.add(item.id);
      setTimeout(renderLibrary, 0);
    });
    return a;
  };
  const parts = item.mirror
    ? [link(item.mirror, `secure copy (${hostOf(item.mirror)})`), " · ", link(item.url, `original (${hostOf(item.url)})`)]
    : [link(item.url, hostOf(item.url))];
  return el("span", { className: "sources" }, "From: ", ...parts);
}

function renderLibrary() {
  const onDisk = new Set(emu.files());
  const installed = (item) => onDisk.has(item.check);
  const count = library.items.filter(installed).length;
  const remaining = library.items.filter((i) => !installed(i));
  const started = remaining.filter((i) => downloaded.has(i.id)).length;
  const notStarted = remaining.filter((i) => !downloaded.has(i.id));
  const toGet = notStarted.reduce((sum, i) => sum + i.size, 0);
  $("lib-counts").textContent = `${library.items.length} programs · ${count} on your disk`;
  // The drop box doubles as the reminder: while downloads are in flight it is
  // highlighted and says how many files to drop.
  const box = $("lib-drop");
  box.classList.toggle("waiting", started > 0);
  box.textContent = started
    ? `Drop the ${started} downloaded file${started === 1 ? "" : "s"} here when ${started === 1 ? "it arrives" : "they arrive"}, or click to choose ${started === 1 ? "it" : "them"}`
    : "Drop downloaded .zip files here, or click to choose them";
  $("lib-all").hidden = notStarted.length === 0;
  $("lib-all").textContent = `Get all (${sizeText(toGet)})`;

  const rows = [];
  for (const group of [...new Set(library.items.map((i) => i.group))]) {
    rows.push(el("tr", { className: "group" }, el("th", { colSpan: 4, scope: "colgroup", textContent: group })));
    for (const item of library.items.filter((i) => i.group === group)) {
      const open = openRows.has(item.id);
      const expand = el("button", { className: "expand", textContent: item.title, title: "How to use it" });
      expand.setAttribute("aria-expanded", String(open));
      expand.addEventListener("click", () => {
        if (open) openRows.delete(item.id);
        else openRows.add(item.id);
        renderLibrary();
      });
      let status;
      if (installed(item)) {
        status = el("span", { className: "on-disk", textContent: "On disk ✓" });
      } else if (failed.has(item.id)) {
        // The site didn't answer: say so, and offer to try again.
        const retry = el("button", { className: "again linkish", textContent: "try again", title: failed.get(item.id) });
        retry.addEventListener("click", () => {
          getPrograms([item]);
          note(`Trying ${item.title} again. Then drop the file on this page.`);
        });
        status = el(
          "span",
          { className: "started failed", title: failed.get(item.id) },
          el("span", { className: "hint", textContent: "Didn't download" }),
          retry,
        );
      } else if (downloaded.has(item.id)) {
        // The page can't see whether the file arrived, so this says what to do
        // next rather than claiming it did, and offers to try again.
        const again = el("button", {
          className: "again linkish",
          textContent: "get again",
          title: "If the file didn't arrive. The program's row also lists its original site.",
        });
        again.addEventListener("click", () => {
          getPrograms([item]);
          note(`Your browser is downloading ${item.title} again. Then drop the file on this page.`);
        });
        status = el(
          "span",
          { className: "started" },
          el("span", { className: "hint", textContent: "Drop it on this page" }),
          again,
        );
      } else {
        status = el("button", {
          className: "get",
          textContent: "Get ↓",
          title: `Download ${decodeURIComponent(item.url.split("/").pop())} from ${hostOf(downloadUrl(item))}`,
        });
      }
      if (status.tagName === "BUTTON") {
        status.addEventListener("click", () => {
          getPrograms([item]);
          note(`Your browser is downloading ${item.title}. Then drop the file on this page.`);
        });
      }
      rows.push(
        el(
          "tr",
          { className: "item" },
          el("td", { className: "name" }, expand),
          el("td", { className: "what", textContent: item.summary }),
          el("td", { className: "size", textContent: sizeText(item.size) }),
          el("td", { className: "status" }, status),
        ),
      );
      if (open) {
        // On a phone the "what it is" column is hidden, so the how-to says it too.
        const what = el("span", { className: "howto-what", textContent: `${item.summary}. ` });
        rows.push(el("tr", { className: "howto" }, el("td", { colSpan: 4 }, what, item.note, sources(item))));
      }
    }
  }
  $("lib-rows").replaceChildren(...rows);
}


// Get all: every program not yet on the disk.
$("lib-all").addEventListener("click", () => {
  const onDisk = new Set(emu.files());
  const items = library.items.filter((i) => !onDisk.has(i.check));
  getPrograms(items);
  const seconds = Math.round((items.length * DOWNLOAD_GAP_MS) / 1000);
  note(
    `Your browser is downloading ${items.length} archives over about ${seconds} seconds. If it ` +
      "asks whether this site may download several files, choose Allow. Then select them all in " +
      "your Downloads folder and drop them on this page together.",
  );
});

// The drop box opens the file picker too, for anyone who'd rather click.
const dropBox = $("lib-drop");
dropBox.addEventListener("click", () => $("add-input").click());
dropBox.addEventListener("keydown", (e) => {
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    $("add-input").click();
  }
});

$("lib-help").addEventListener("click", (e) => {
  e.preventDefault();
  const help = $("help-adding");
  help.open = true;
  help.scrollIntoView({ behavior: "smooth", block: "start" });
});

onClick("fresh-disk", () => {
  if (!confirm("Erase drive A: and start again with only the bundled programs? Save the disk first to keep your files.")) return;
  swapEmulator(new Emulator(), true);
  note("Fresh disk.");
});

// ---------------------------------------------------------------------------
// Machine

onClick("restart", () => {
  if (emu.inProgram() && !confirm(ENDS_PROGRAM + " Restart anyway?")) return;
  swapEmulator(Emulator.fromImage(emu.diskImage()), false);
  note("CP/M restarted; drive A: kept its files.");
});
speedSelect.addEventListener("change", () => {
  speed = speedSelect.value;
  storeSpeed(speed);
  showStatus();
  term.focus();
});

renderWelcome();
wake();
term.focus();
