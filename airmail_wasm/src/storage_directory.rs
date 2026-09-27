//! A read-only tantivy `Directory` that reads through synchronous JS globals
//! (`airmailStorageExists`/`Len`/`ReadRange`), backed by OPFS in a Worker or `fs` in Node.

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
        // The default impl writes a lock file, which fails here; with no
        // writers possible, granting every lock is safe.
        Ok(DirectoryLock::from(Box::new(|| {})))
    }

    fn watch(&self, _watch_callback: WatchCallback) -> tantivy::Result<WatchHandle> {
        Ok(WatchHandle::empty())
    }
}
