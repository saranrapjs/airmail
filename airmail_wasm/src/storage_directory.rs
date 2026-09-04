//! A read-only tantivy `Directory` backed by whatever synchronous
//! byte-storage the host JS environment provides, via three global
//! functions: `airmailStorageExists`, `airmailStorageLen`, and
//! `airmailStorageReadRange`.
//!
//! This indirection is deliberate: tantivy's `Directory`/`FileHandle` API is
//! synchronous, and so is the browser's OPFS `FileSystemSyncAccessHandle` --
//! but `FileSystemSyncAccessHandle` only exists inside a dedicated Worker,
//! which makes it awkward to exercise directly in an automated test. By
//! putting a thin, synchronous JS function boundary in between, the *same*
//! compiled wasm module can be driven by two different hosts: a Node.js
//! script (using synchronous `fs` calls) for fast, no-browser-required
//! testing of the Rust-side Directory logic, and, unmodified, a real Worker
//! (using `FileSystemSyncAccessHandle`) for production. Only the three JS
//! functions differ between the two; nothing on the Rust side changes.
//!
//! It's read-only (`delete`/`open_write`/`atomic_write` all fail) because
//! the only supported use case is opening a pre-built index for search --
//! never building or modifying one in the browser.

use std::fmt;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tantivy::directory::error::{DeleteError, LockError, OpenReadError, OpenWriteError};
use tantivy::directory::{Directory, DirectoryLock, FileHandle, Lock, WatchCallback, WatchHandle, WritePtr};
use tantivy_common::{HasLen, OwnedBytes};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = airmailStorageExists)]
    fn storage_exists(path: &str) -> bool;

    #[wasm_bindgen(js_namespace = globalThis, js_name = airmailStorageLen, catch)]
    fn storage_len(path: &str) -> Result<f64, JsValue>;

    #[wasm_bindgen(js_namespace = globalThis, js_name = airmailStorageReadRange, catch)]
    fn storage_read_range(path: &str, start: f64, end: f64) -> Result<Vec<u8>, JsValue>;
}

fn js_err_to_io(path: &Path, err: JsValue) -> io::Error {
    io::Error::other(format!("{path:?}: {err:?}"))
}

#[derive(Clone)]
pub struct StorageDirectory;

impl StorageDirectory {
    pub fn new() -> Self {
        Self
    }
}

impl fmt::Debug for StorageDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StorageDirectory")
    }
}

#[derive(Debug)]
struct StorageFileHandle {
    path: PathBuf,
    path_str: String,
    len: usize,
}

impl HasLen for StorageFileHandle {
    fn len(&self) -> usize {
        self.len
    }
}

impl FileHandle for StorageFileHandle {
    fn read_bytes(&self, range: Range<usize>) -> io::Result<OwnedBytes> {
        let bytes = storage_read_range(&self.path_str, range.start as f64, range.end as f64)
            .map_err(|e| js_err_to_io(&self.path, e))?;
        Ok(OwnedBytes::new(bytes))
    }
}

impl Directory for StorageDirectory {
    fn get_file_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
        let path_str = path.to_string_lossy().to_string();
        if !storage_exists(&path_str) {
            return Err(OpenReadError::FileDoesNotExist(path.to_path_buf()));
        }
        let len = storage_len(&path_str)
            .map_err(|e| OpenReadError::wrap_io_error(js_err_to_io(path, e), path.to_path_buf()))?
            as usize;
        Ok(Arc::new(StorageFileHandle {
            path: path.to_path_buf(),
            path_str,
            len,
        }))
    }

    fn exists(&self, path: &Path) -> Result<bool, OpenReadError> {
        Ok(storage_exists(&path.to_string_lossy()))
    }

    fn atomic_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        let handle = self.get_file_handle(path)?;
        handle
            .read_bytes(0..handle.len())
            .map(|bytes| bytes.as_slice().to_vec())
            .map_err(|e| OpenReadError::wrap_io_error(e, path.to_path_buf()))
    }

    fn delete(&self, path: &Path) -> Result<(), DeleteError> {
        Err(DeleteError::IoError {
            io_error: Arc::new(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "StorageDirectory is read-only",
            )),
            filepath: path.to_path_buf(),
        })
    }

    fn open_write(&self, path: &Path) -> Result<WritePtr, OpenWriteError> {
        Err(OpenWriteError::wrap_io_error(
            io::Error::new(io::ErrorKind::PermissionDenied, "StorageDirectory is read-only"),
            path.to_path_buf(),
        ))
    }

    fn atomic_write(&self, _path: &Path, _data: &[u8]) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "StorageDirectory is read-only",
        ))
    }

    fn sync_directory(&self) -> io::Result<()> {
        Ok(())
    }

    fn acquire_lock(&self, _lock: &Lock) -> Result<DirectoryLock, LockError> {
        // The trait's default impl shells out to open_write/delete to
        // create a real lock file on disk, which fails on this read-only
        // directory. But nothing else can concurrently open an IndexWriter
        // against this directory in the first place (there's no writer path
        // at all here), so trivially granting every lock is safe -- there's
        // nothing to actually protect against.
        Ok(DirectoryLock::from(Box::new(|| {})))
    }

    fn watch(&self, _watch_callback: WatchCallback) -> tantivy::Result<WatchHandle> {
        Ok(WatchHandle::empty())
    }
}
