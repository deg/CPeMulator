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
// browser copy is a convenience, and the downloaded .dsk is the user's real
// copy. The page tracks whether the disk has changed since it was last
// downloaded ("backed up"), keeps that flag in localStorage, and says so in the
// status line. (localStorage, not IndexedDB, because it writes synchronously:
// an IndexedDB write started by a click on Download is lost if the tab closes
// or reloads a moment later. Either store being cleared alone errs safe: no
// flag reads as "not backed up", and no image boots a fresh disk.) It also asks the browser to keep the data (persist()), which the
// browser may refuse.
let backedUp = true; // has drive A: been downloaded since it last changed?
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
}
// Called after every slice: the first change to a backed-up disk clears the
// flag, and is also the moment there is something worth asking the browser to
// keep.
function noteDiskChange() {
  if (!backedUp || emu.diskChanges() === baseline) return;
  backedUp = false;
  storeBackedUp();
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
    saveState = "disk saved in this browser";
  } catch (e) {
    saveState = `disk NOT saved (${e?.name ?? e})`;
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
saveState = db ? "disk saved in this browser" : "disk not saved (browser storage unavailable)";

// Declared before anything can call showStatus(), which reads them.
let state = "running"; // the last emu.run() result
let speed = loadSpeed();
const speedSelect = $("m-speed");
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
    if (!$("files").hidden) renderFiles();
  }
  showStatus();
}

function showStatus() {
  const clock = speed === "max" ? "full speed" : `${speed} MHz`;
  const what = {
    running: "running",
    idle: "waiting for a key",
  }[state] ?? `stopped (${state}): press Reset`;
  const backup = backedUp ? "" : "changed since your last Download disk";
  const parts = [statusNote, `${clock} Z80 · ${what}`, saveState, backup].filter(Boolean);
  const el = $("status");
  el.textContent = parts.join(" · ");
  el.classList.toggle("error", statusError || !(state === "running" || state === "idle"));
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
  term.reset();
  state = "running";
  wake();
  if (!$("files").hidden) renderFiles();
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
onClick("k-ctrlc", () => send(new Uint8Array([0x03])));
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
      note("Upload a .dsk image on its own: it replaces the whole disk.", true);
      return;
    }
    const f = images[0];
    if (!confirm(`Replace drive A: with ${f.name}? Its current files are lost unless you download it first.`)) return;
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
  if (emu.inProgram() && !confirm(ENDS_PROGRAM + " Upload anyway?")) return;
  const done = [];
  const failed = [];
  for (const f of files) {
    try {
      emu.putFile(f.name, new Uint8Array(await f.arrayBuffer()));
      done.push(f.name.toUpperCase());
    } catch (e) {
      failed.push(`${f.name}: ${e.message ?? e}`);
    }
  }
  if (done.length) {
    // putFile warm-boots CP/M so it sees the new files; show its fresh prompt.
    state = "running";
    wake();
    if (!$("files").hidden) renderFiles();
  }
  const msg = [done.length ? `Put on A: ${done.join(", ")}.` : "", ...failed].filter(Boolean).join(" ");
  note(msg, failed.length > 0);
}

const fileInput = $("d-input");
onClick("d-upload", () => fileInput.click());
fileInput.addEventListener("change", () => {
  putFiles(fileInput.files);
  fileInput.value = "";
});

const termBox = $("term");
termBox.addEventListener("dragover", (e) => {
  e.preventDefault();
  termBox.classList.add("dragging");
});
termBox.addEventListener("dragleave", () => termBox.classList.remove("dragging"));
termBox.addEventListener("drop", (e) => {
  e.preventDefault();
  termBox.classList.remove("dragging");
  putFiles(e.dataTransfer.files);
});

onClick("d-download", () => {
  download(emu.diskImage(), "cpemulator-A.dsk");
  setBackedUp(true);
  note("Downloaded drive A: as cpemulator-A.dsk. Upload it to bring it back.");
});

function renderFiles() {
  const names = emu.files().sort();
  $("files-count").textContent = `${names.length} file${names.length === 1 ? "" : "s"}`;
  const list = $("files-list");
  list.replaceChildren(
    ...names.map((name) => {
      const li = document.createElement("li");
      const b = document.createElement("button");
      b.textContent = name;
      b.title = `Download ${name}`;
      b.addEventListener("click", () => {
        const bytes = emu.readFile(name);
        if (bytes) download(bytes, name);
      });
      li.append(b);
      return li;
    }),
  );
}
onClick("d-files", () => {
  const panel = $("files");
  panel.hidden = !panel.hidden;
  $("d-files").setAttribute("aria-expanded", String(!panel.hidden));
  if (!panel.hidden) renderFiles();
});
onClick("d-fresh", () => {
  if (!confirm("Erase drive A: and start again with only the bundled programs? Download it first to keep your files.")) return;
  swapEmulator(new Emulator(), true);
  note("Fresh disk.");
});

// ---------------------------------------------------------------------------
// Machine

onClick("m-reset", () => {
  if (emu.inProgram() && !confirm(ENDS_PROGRAM + " Reset anyway?")) return;
  swapEmulator(Emulator.fromImage(emu.diskImage()), false);
  note("Reset: CP/M restarted; drive A: kept its files.");
});
speedSelect.addEventListener("change", () => {
  speed = speedSelect.value;
  storeSpeed(speed);
  showStatus();
  term.focus();
});

wake();
term.focus();
