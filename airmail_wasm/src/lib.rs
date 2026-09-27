mod storage_directory;

use airmail::index::AirmailIndex;
use storage_directory::StorageDirectory;
use wasm_bindgen::prelude::*;

/// Runs a search against an empty in-memory index, keeping the full search
/// path linked so measured .wasm size is realistic.
#[wasm_bindgen]
pub async fn smoke_test(query: String) -> Result<usize, JsValue> {
    let index = AirmailIndex::create_in_ram().map_err(|e| JsValue::from_str(&e.to_string()))?;
    let results = index
        .search(&query, true, None, None, &[])
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(results.len())
}

/// A pre-built index opened through `StorageDirectory`'s JS storage functions.
#[wasm_bindgen]
pub struct IndexHandle(AirmailIndex);

#[wasm_bindgen]
pub fn open_index() -> Result<IndexHandle, JsValue> {
    let index =
        AirmailIndex::open(StorageDirectory::new()).map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(IndexHandle(index))
}

/// Returns JSON `[(AirmailPoi, score), ...]`, the same shape as airmail_service's `/search`.
#[wasm_bindgen]
pub async fn search_index(handle: &IndexHandle, query: String) -> Result<String, JsValue> {
    let results = handle
        .0
        .search(&query, true, None, None, &[])
        .await
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    serde_json::to_string(&results).map_err(|e| JsValue::from_str(&e.to_string()))
}
