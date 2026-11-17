// Smoke test for the browser build: load the built wasm in Node, which runs the
// same module a browser does, and boot CP/M. Native `cargo test` covers the
// emulation; this catches what only the wasm build can get wrong, such as a std
// call that compiles for wasm32 but panics when run (the clock, sleeping).
//
//   web/build.sh && node web/smoke.mjs
//
// Exits non-zero on any failure.

import { readFileSync } from "node:fs";
import init, { Emulator } from "./www/pkg/cpemu_web.js";

const wasm = readFileSync(new URL("./www/pkg/cpemu_web_bg.wasm", import.meta.url));
await init({ module_or_path: wasm });

const FRAME = 66_667; // T-states in one 60 Hz frame at 4 MHz
const decoder = new TextDecoder();
let failures = 0;

// Run frame-sized slices until CP/M waits for a key; return the screen text.
function runToIdle(emu) {
  let out = "";
  for (let frame = 0; frame < 1000; frame++) {
    const state = emu.run(FRAME);
    out += decoder.decode(emu.takeOutput());
    if (state === "idle") return out;
    if (state !== "running") throw new Error(`machine stopped: ${state}`);
  }
  throw new Error(`never went idle; output so far: ${JSON.stringify(out)}`);
}

function check(name, fn) {
  try {
    const problem = fn();
    if (problem) {
      console.log(`FAIL ${name}: ${problem}`);
      failures++;
    } else {
      console.log(`ok   ${name}`);
    }
  } catch (e) {
    // A Rust panic traps and leaves the instance unusable.
    console.log(`FAIL ${name}: ${e}`);
    failures++;
  }
}

const emu = new Emulator();
check("boots to the A> prompt", () => {
  const out = runToIdle(emu);
  return out.includes("A>") ? null : `no prompt in ${JSON.stringify(out)}`;
});
check("runs a typed DIR, listing the bundled utilities", () => {
  emu.typeBytes(new TextEncoder().encode("dir\r"));
  const out = runToIdle(emu);
  return out.includes("STAT     COM") ? null : `unexpected output ${JSON.stringify(out)}`;
});
check("idle costs nothing", () => {
  const before = emu.cycles();
  const state = emu.run(FRAME);
  return state === "idle" && emu.cycles() === before ? null : `state ${state}, ran ${emu.cycles() - before}`;
});

// A tiny .COM that prints "HI" through BDOS function 9 and returns to the CCP.
const HI = new Uint8Array([0x0e, 0x09, 0x11, 0x09, 0x01, 0xcd, 0x05, 0x00, 0xc9, 0x48, 0x49, 0x24]);
check("an uploaded program runs", () => {
  emu.putFile("hi.com", HI);
  runToIdle(emu); // the warm boot's fresh prompt
  emu.typeBytes(new TextEncoder().encode("hi\r"));
  const out = runToIdle(emu);
  return out.includes("HI") ? null : `unexpected output ${JSON.stringify(out)}`;
});
check("a bad file name is a JS error, not a trap", () => {
  try {
    emu.putFile("my file.com", HI);
  } catch (e) {
    return String(e).includes("not a CP/M file name") ? null : `wrong error: ${e}`;
  }
  return "accepted a name with a space";
});
check("a saved disk image boots with its files", () => {
  const image = emu.diskImage();
  const again = Emulator.fromImage(image);
  runToIdle(again);
  again.typeBytes(new TextEncoder().encode("dir\r"));
  const out = runToIdle(again);
  return out.includes("HI       COM") && again.files().includes("HI.COM")
    ? null : `files ${again.files()}, output ${JSON.stringify(out)}`;
});
check("a wrong-sized image is refused", () => {
  try {
    Emulator.fromImage(new Uint8Array(1000));
  } catch (e) {
    return String(e).includes("not a CPeMulator disk image") ? null : `wrong error: ${e}`;
  }
  return "accepted a 1000-byte image";
});

check("a .zip's files go on the disk, bad names reported", () => {
  const zip = readFileSync(new URL("../crates/web/tests/fixtures/sample.zip", import.meta.url));
  const report = emu.putZip(zip, [], []);
  const put = report.put.join(",");
  const skipped = report.skipped.join(",");
  if (put !== "HELLO.COM,README.TXT") return `put ${put}`;
  if (!skipped.startsWith("bad name.txt: ")) return `skipped ${skipped}`;
  runToIdle(emu);
  emu.typeBytes(new TextEncoder().encode("hello\r"));
  const out = runToIdle(emu);
  return out.includes("HELLO FROM A ZIP") ? null : `unexpected output ${JSON.stringify(out)}`;
});
check("something that isn't a zip is a JS error", () => {
  try {
    emu.putZip(new Uint8Array([1, 2, 3]), [], []);
  } catch (e) {
    return String(e).includes("not a zip archive") ? null : `wrong error: ${e}`;
  }
  return "accepted";
});

// The page's software library (web/www/library.json) repeats the desktop's
// (scripts/fetch-software.sh) as links; keep the two in step.
check("the web library matches the desktop's", () => {
  const lib = JSON.parse(readFileSync(new URL("./www/library.json", import.meta.url), "utf8"));
  const sh = readFileSync(new URL("../scripts/fetch-software.sh", import.meta.url), "utf8");
  const base = sh.match(/^base="([^"]+)"/m)[1];
  const desktop = new Set([
    ...[...sh.matchAll(/^fetch "\$base\/([^"]+)"/gm)].map((m) => `${base}/${m[1]}`),
    sh.match(/^wsurl="([^"]+)"/m)[1],
    ...[...sh.matchAll(/"(https:\/\/www\.bdsoft\.com\/[^"]+)"/g)].map((m) => m[1]),
  ]);
  // DRI's own utilities are already on a fresh disk.
  desktop.delete(`${base}/os/STDCPM22.ZIP`);
  const web = new Set(lib.items.map((i) => i.url));
  const missing = [...desktop].filter((u) => !web.has(u));
  const extra = [...web].filter((u) => !desktop.has(u));
  const rules = [
    ...sh.match(/^EXTRAS="([^"]*)"/m)[1].replace(/\\\n/g, " ").split(/\s+/),
    ...sh.match(/^NOISE="([^"]*)"/m)[1].split(/\s+/),
  ].filter(Boolean);
  const sameRules = rules.length === lib.exclude.length && rules.every((r, i) => r === lib.exclude[i]);
  if (missing.length || extra.length) return `missing ${missing}; not in the desktop's: ${extra}`;
  // A mirror is a Wayback Machine capture of the entry's own URL, over HTTPS.
  const badMirror = lib.items.filter(
    (i) => i.mirror && !(i.mirror.startsWith("https://web.archive.org/web/") && i.mirror.endsWith(`id_/${i.url}`)),
  );
  if (badMirror.length) return `mirrors not matching their URL: ${badMirror.map((i) => i.id)}`;
  const insecure = lib.items.filter((i) => !(i.mirror ?? i.url).startsWith("https://"));
  if (insecure.length) return `downloaded over plain HTTP: ${insecure.map((i) => i.id)}`;
  return sameRules ? null : "exclude rules differ from EXTRAS + NOISE";
});
check("zipPaths lists an archive's files without unpacking it", () => {
  const zip = readFileSync(new URL("../crates/web/tests/fixtures/sample.zip", import.meta.url));
  const paths = Emulator.zipPaths(zip).join(",");
  return paths === "GAME/HELLO.COM,readme.txt,bad name.txt,__MACOSX/GAME/._HELLO.COM" ? null : paths;
});
check("an archive's rules leave files out", () => {
  const zip = readFileSync(new URL("../crates/web/tests/fixtures/sample.zip", import.meta.url));
  const report = emu.putZip(zip, ["GAME/"], ["*.txt"]);
  const put = report.put.join(",");
  return put === "HELLO.COM" && report.leftOut === 2 && report.skipped.length === 0
    ? null : `put ${put}, left out ${report.leftOut}, skipped ${report.skipped}`;
});

if (failures) {
  console.log(`${failures} failure(s)`);
  process.exit(1);
}
console.log("all ok");
