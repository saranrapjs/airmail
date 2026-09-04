// Standalone Node.js test for the wasm-bindgen `airmail_wasm` module's
// StorageDirectory-backed index opening/search path (POC step 4, minus the
// browser/OPFS part -- see storage_directory.rs's doc comment for why a
// Node harness can stand in for the browser here).
//
// Usage: node airmail_wasm/test/node_test.cjs <index-dir> <query>

const fs = require("fs");
const path = require("path");

const indexDir = process.argv[2];
const query = process.argv[3];
if (!indexDir || !query) {
  console.error("usage: node node_test.cjs <index-dir> <query>");
  process.exit(1);
}

// The three sync storage primitives storage_directory.rs calls into.
// Paths passed in by tantivy are relative (e.g. "meta.json",
// "<segment>.idx"); resolve them against indexDir.
globalThis.airmailStorageExists = (relPath) => {
  return fs.existsSync(path.join(indexDir, relPath));
};

globalThis.airmailStorageLen = (relPath) => {
  return fs.statSync(path.join(indexDir, relPath)).size;
};

globalThis.airmailStorageReadRange = (relPath, start, end) => {
  const fd = fs.openSync(path.join(indexDir, relPath), "r");
  try {
    const buf = Buffer.alloc(end - start);
    fs.readSync(fd, buf, 0, end - start, start);
    return new Uint8Array(buf);
  } finally {
    fs.closeSync(fd);
  }
};

async function main() {
  const wasm = require("../pkg-node/airmail_wasm.js");
  const handle = wasm.open_index();
  const resultsJson = await wasm.search_index(handle, query);
  const results = JSON.parse(resultsJson);
  console.log(`${results.length} result(s) for ${JSON.stringify(query)}:`);
  for (const [poi, score] of results) {
    console.log(`  [${score.toFixed(3)}] ${poi.source} (${poi.lat}, ${poi.lng})`);
  }
}

main().catch((err) => {
  console.error("FAILED:", err);
  process.exit(1);
});
