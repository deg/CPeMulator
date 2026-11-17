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

// Which library archives this browser has started downloading, so their
// buttons say so after a reload. (The page can't see whether a download
// finished; "Installed" comes from the disk itself.)
function loadDownloaded() {
  try {
    return new Set(JSON.parse(localStorage.getItem("cpemulator.library.downloaded") ?? "[]"));
  } catch {
    return new Set();
  }
}
function storeDownloaded() {
  try {
    localStorage.setItem("cpemulator.library.downloaded", JSON.stringify([...downloaded]));
  } catch {
    // Not remembered; harmless.
  }
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
const downloaded = loadDownloaded();
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
    if (!$("files").hidden) renderFiles();
    if (!$("library").hidden) renderLibrary();
    runSetup();
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
  setup = null; // a half-run install belonged to the old machine
  term.reset();
  state = "running";
  wake();
  if (!$("files").hidden) renderFiles();
  if (!$("library").hidden) renderLibrary();
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
    if (!$("files").hidden) renderFiles();
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
// The library table: a heading row per group, then per program its name
// (which opens a how-to row), what it is, its download size, and a status:
// Download, then Downloaded once clicked, then Installed once its program is on
// the disk.
const openRows = new Set(); // ids whose how-to is showing

function sizeText(bytes) {
  return bytes >= 1e6 ? `${(bytes / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round(bytes / 1e3))} KB`;
}

function el(tag, props = {}, ...children) {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children);
  return e;
}

function startDownload(item) {
  // A plain link: the archive sites send .zip files as downloads, so the page
  // stays where it is.
  const a = el("a", { href: item.url, rel: "noopener" });
  document.body.append(a);
  a.click();
  a.remove();
  downloaded.add(item.id);
  storeDownloaded();
}

function renderLibrary() {
  const onDisk = new Set(emu.files());
  const installed = (item) => onDisk.has(item.check);
  const count = library.items.filter(installed).length;
  const remaining = library.items.filter((i) => !installed(i));
  const toGet = remaining.reduce((sum, i) => sum + i.size, 0);
  $("lib-counts").textContent =
    `${library.items.length} programs · ${count} installed` +
    (remaining.length ? ` · ${sizeText(toGet)} to download` : "");
  $("lib-all").hidden = remaining.length === 0;

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
        status = el("span", { className: "installed", textContent: "Installed ✓" });
      } else if (downloaded.has(item.id)) {
        status = el("button", {
          className: "get downloaded",
          textContent: "Downloaded",
          title: "Drop the file on the screen to install it. Click to download it again.",
        });
      } else {
        status = el("button", {
          className: "get primary",
          textContent: "Download ↓",
          title: `Download ${decodeURIComponent(item.url.split("/").pop())} from ${new URL(item.url).hostname}`,
        });
      }
      if (status.tagName === "BUTTON") {
        status.addEventListener("click", () => {
          startDownload(item);
          note(`Downloading ${item.title}. Then drop the file on the screen.`);
          renderLibrary();
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
        rows.push(el("tr", { className: "howto" }, el("td", { colSpan: 4 }, what, item.note)));
      }
    }
  }
  $("lib-rows").replaceChildren(...rows);
}

onClick("d-library", () => {
  const panel = $("library");
  panel.hidden = !panel.hidden;
  $("d-library").setAttribute("aria-expanded", String(!panel.hidden));
  if (!panel.hidden) renderLibrary();
});

// Download every program not yet on the disk, a moment apart (browsers treat a
// burst of downloads as suspect, and ask once whether to allow several).
$("lib-all").addEventListener("click", () => {
  const onDisk = new Set(emu.files());
  const items = library.items.filter((i) => !onDisk.has(i.check));
  items.forEach((item, k) => setTimeout(() => startDownload(item), k * 700));
  setTimeout(renderLibrary, items.length * 700);
  note(
    `Downloading ${items.length} archives. Your browser may ask to allow several downloads, ` +
      "and to Keep files from retroarchive.org. Then select them all in your Downloads folder " +
      "and drop them on the screen together.",
  );
});

$("lib-help").addEventListener("click", (e) => {
  e.preventDefault();
  const help = $("help-adding");
  help.open = true;
  help.scrollIntoView({ behavior: "smooth", block: "start" });
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
