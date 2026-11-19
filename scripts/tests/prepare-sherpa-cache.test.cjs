const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { test } = require("node:test");

const libraryNames = [
  "sherpa-onnx-c-api", "sherpa-onnx-core", "kaldi-decoder-core",
  "sherpa-onnx-kaldifst-core", "sherpa-onnx-fstfar", "sherpa-onnx-fst",
  "kaldi-native-fbank-core", "kissfft-float", "piper_phonemize", "espeak-ng",
  "ucd", "onnxruntime", "ssentencepiece_core",
];

async function fixture(t) {
  const { prepareSherpaCache } = await import("../prepare-sherpa-cache.mjs");
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "sherpa-cache-test-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const target = path.join(root, "target");
  const cache = path.join(target, "sherpa-onnx-prebuilt");
  const temporary = path.join(root, "temporary");
  return { root, target, cache, temporary, run: () => prepareSherpaCache(target, temporary) };
}

for (const platform of ["win-x64-static-MT-Release", "linux-x64-static"]) {
  for (const missing of [null, "sherpa-onnx-c-api", "onnxruntime"]) {
    test(`${platform}: ${missing ? `quarantines missing ${missing}` : "keeps a complete package"}`, async t => {
      const f = await fixture(t);
      const stem = `sherpa-onnx-v1.13.8-${platform}-lib`;
      const extracted = path.join(f.cache, stem);
      const lib = path.join(extracted, "lib");
      fs.mkdirSync(lib, { recursive: true });
      for (const name of libraryNames) {
        if (name === missing) continue;
        fs.writeFileSync(path.join(lib, platform.startsWith("win") ? `${name}.lib` : `lib${name}.a`), "library");
      }
      const archive = `${extracted}.tar.bz2`;
      fs.writeFileSync(archive, "cached archive");
      assert.deepEqual(f.run(), missing ? [stem] : []);
      assert.equal(fs.existsSync(extracted), !missing);
      assert.equal(fs.existsSync(archive), !missing);
      if (missing) {
        const quarantine = path.join(f.temporary, fs.readdirSync(f.temporary)[0]);
        assert.equal(fs.readFileSync(path.join(quarantine, `${stem}.tar.bz2`), "utf8"), "cached archive");
        assert.equal(fs.existsSync(path.join(quarantine, stem, "lib")), true);
        assert.deepEqual(f.run(), [], "repeated preparation is harmless");
      }
    });
  }
}

test("an absent prebuilt cache does not create directories", async t => {
  const f = await fixture(t);
  assert.deepEqual(f.run(), []);
  assert.equal(fs.existsSync(f.target), false);
  assert.equal(fs.existsSync(f.temporary), false);
});

test("unrelated packages and Cargo artifacts are preserved", async t => {
  const f = await fixture(t);
  const unrelated = path.join(f.cache, "other-library", "keep");
  const artifact = path.join(f.target, "debug", "keep");
  for (const file of [unrelated, artifact]) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, "preserved");
  }
  assert.deepEqual(f.run(), []);
  for (const file of [unrelated, artifact]) assert.equal(fs.readFileSync(file, "utf8"), "preserved");
});

test("a junction cannot redirect a sherpa package outside the cache", async t => {
  const f = await fixture(t);
  const outside = path.join(f.root, "outside");
  fs.mkdirSync(f.cache, { recursive: true });
  fs.mkdirSync(outside);
  fs.writeFileSync(path.join(outside, "keep"), "preserved");
  const link = path.join(f.cache, "sherpa-onnx-v1.13.8-win-x64-static-MT-Release-lib");
  fs.symlinkSync(outside, link, process.platform === "win32" ? "junction" : "dir");
  assert.throws(f.run, /symlinked sherpa package/);
  assert.equal(fs.readFileSync(path.join(outside, "keep"), "utf8"), "preserved");
});

test("a junction cannot redirect the cache root", async t => {
  const f = await fixture(t);
  const outside = path.join(f.root, "outside");
  fs.mkdirSync(f.target, { recursive: true });
  fs.mkdirSync(outside);
  fs.symlinkSync(outside, f.cache, process.platform === "win32" ? "junction" : "dir");
  assert.throws(f.run, /symlinked sherpa cache/);
  assert.equal(fs.existsSync(outside), true);
});

test("the CLI honors the target and runner temporary directories", async t => {
  const f = await fixture(t);
  const stem = "sherpa-onnx-v1.13.8-linux-x64-static-lib";
  const target = path.join(f.root, "custom-target");
  const cache = path.join(target, "sherpa-onnx-prebuilt");
  fs.mkdirSync(path.join(cache, stem, "lib"), { recursive: true });
  const result = spawnSync(process.execPath, [path.join(__dirname, "../prepare-sherpa-cache.mjs")], {
    cwd: f.root,
    env: { ...process.env, CARGO_TARGET_DIR: target, RUNNER_TEMP: f.temporary },
    encoding: "utf8", windowsHide: true, timeout: 10000,
  });
  assert.ifError(result.error);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(fs.existsSync(path.join(cache, stem)), false);
  assert.equal(fs.existsSync(f.target), false);
  assert.equal(fs.readdirSync(f.temporary).length, 1);
});

test("both CI jobs repair caches and invalidate build scripts before Rust tests", () => {
  const workflow = fs.readFileSync(path.join(__dirname, "../../.github/workflows/pr-build.yml"), "utf8");
  for (const job of ["windows-artifacts", "linux-deb"]) {
    const body = workflow.split(`  ${job}:`)[1].split(/\n  [a-z][\w-]*:/)[0];
    const cache = body.indexOf("- name: Cache Rust dependencies");
    const prepare = body.indexOf("- name: Check sherpa-onnx prebuilt cache");
    const tests = body.indexOf("- name: Rust tests");
    assert.ok(cache >= 0 && prepare > cache && tests > prepare, `${job}: prepare before compilation`);
    const step = body.slice(prepare).split(/\n      - name:/)[0];
    assert.match(step, /shell: bash/);
    assert.match(step, /node --test scripts\/tests\/prepare-sherpa-cache\.test\.cjs/);
    assert.match(step, /node scripts\/prepare-sherpa-cache\.mjs/);
    assert.match(step, /cargo clean --package sherpa-onnx-sys/);
    assert.doesNotMatch(step, /\n        if:/, "cache recovery also runs when only build-script outputs were restored");
  }
});
