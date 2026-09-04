mod storage_directory;

use airmail::index::AirmailIndex;
use storage_directory::StorageDirectory;
use wasm_bindgen::prelude::*;

/// Exercises the real schema/query/search/result-building code paths (the
/// same ones a real browser search would hit) against an empty in-memory
/// index, so `wasm-opt`-measured binary size reflects what actually ships,
/// not a stub.
#[wasm_bindgen]
pub async fn smoke_test(query: String) -> Result<usize, JsValue> {
    let index = AirmailIndex::create_in_ram().map_err(|e| JsValue::from_str(&e.to_string()))?;
    let results = index
        .search(&query, true, None, None, &[])
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(results.len())
}

/// Opens a real, pre-built index directory via `StorageDirectory`, which
/// reads bytes through three JS-provided global functions
/// (`airmailStorageExists`/`airmailStorageLen`/`airmailStorageReadRange`).
/// In a real browser deployment those would be backed by OPFS
/// (`FileSystemSyncAccessHandle`, inside a Worker); in this POC's Node.js
/// test harness they're backed by synchronous `fs` reads instead -- the
/// Rust side is identical either way.
#[wasm_bindgen]
pub struct IndexHandle(AirmailIndex);

#[wasm_bindgen]
pub fn open_index() -> Result<IndexHandle, JsValue> {
    let index =
        AirmailIndex::open(StorageDirectory::new()).map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(IndexHandle(index))
}

/// Returns JSON-encoded `[(AirmailPoi, f32 score), ...]`, matching what
/// `airmail_service`'s `/search` endpoint (`airmail::index::AirmailIndex::search`)
/// returns for the same query, so results can be diffed against a native
/// run for correctness.
#[wasm_bindgen]
pub async fn search_index(handle: &IndexHandle, query: String) -> Result<String, JsValue> {
    let results = handle
        .0
        .search(&query, true, None, None, &[])
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_json::to_string(&results).map_err(|e| JsValue::from_str(&e.to_string()))
}
