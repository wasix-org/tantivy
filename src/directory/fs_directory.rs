use std::{
    fs::File,
    io::{Read, Seek, Write},
    ops::Range,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use common::{file_slice::FileHandle, HasLen, OwnedBytes, TerminatingWrite};

use super::error::{DeleteError, OpenDirectoryError, OpenReadError, OpenWriteError};
use super::{file_watcher::FileWatcher, WatchCallback, WatchHandle, WritePtr};

#[derive(Debug)]
struct FileWriter(File);

impl std::io::Write for FileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

impl TerminatingWrite for FileWriter {
    fn terminate_ref(&mut self, _: common::AntiCallToken) -> std::io::Result<()> {
        self.0.flush()
    }
}

#[derive(Debug)]
pub struct FsFileHandle {
    file: Mutex<File>,
    length: usize,
}

impl HasLen for FsFileHandle {
    fn len(&self) -> usize {
        self.length
    }
}

impl FileHandle for FsFileHandle {
    fn read_bytes(&self, range: Range<usize>) -> std::io::Result<OwnedBytes> {
        let mut file = self.file.lock().unwrap();
        file.seek(std::io::SeekFrom::Start(range.start.try_into().unwrap()))?;
        let mut buffer = vec![0u8; range.len()];
        file.read_exact(&mut buffer)?;
        Ok(OwnedBytes::new(buffer))
    }
}

/// Directory backed by a filesystem
#[derive(Clone)]
pub struct FsDirectory {
    root: PathBuf,
    watcher: Arc<FileWatcher>,
}

impl FsDirectory {
    /// Opens a diectory at `path`
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, OpenDirectoryError> {
        let path = path.as_ref();

        std::fs::create_dir_all(path).unwrap();

        let watcher = Arc::new(FileWatcher::new(&path.join(*crate::core::META_FILEPATH)));

        Ok(Self {
            root: path.to_owned(),
            watcher,
        })
    }

    fn prepare_path(&self, path: &Path) -> PathBuf {
        self.root.join(path)
    }
}

impl std::fmt::Debug for FsDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FsDirectory")
            .field("root", &self.root)
            .finish()
    }
}

impl super::Directory for FsDirectory {
    fn get_file_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .open(self.prepare_path(path))
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    OpenReadError::FileDoesNotExist(path.to_owned())
                } else {
                    OpenReadError::wrap_io_error(err, path.to_owned())
                }
            })?;

        let length: usize = file
            .metadata()
            .map_err(|e| OpenReadError::wrap_io_error(e, path.to_owned()))?
            .len()
            .try_into()
            .unwrap();

        Ok(Arc::new(FsFileHandle {
            file: Mutex::new(file),
            length,
        }))
    }

    fn delete(&self, path: &Path) -> Result<(), DeleteError> {
        std::fs::remove_file(self.prepare_path(path)).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                DeleteError::FileDoesNotExist(path.to_owned())
            } else {
                DeleteError::IoError {
                    io_error: Arc::new(err),
                    filepath: path.to_owned(),
                }
            }
        })
    }

    fn exists(&self, path: &Path) -> Result<bool, OpenReadError> {
        match std::fs::metadata(self.prepare_path(path)) {
            Ok(m) => Ok(m.is_file()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                Ok(false)
            }
            Err(err) => Err(OpenReadError::wrap_io_error(err, path.to_owned())),
        }
    }

    fn open_write(&self, path: &Path) -> Result<WritePtr, OpenWriteError> {
        let full_path = self.prepare_path(path);

        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&full_path)
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    OpenWriteError::FileAlreadyExists(full_path)
                } else {
                    OpenWriteError::wrap_io_error(err, full_path)
                }
            })?;

        Ok(super::WritePtr::new(Box::new(FileWriter(file))))
    }

    fn atomic_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        std::fs::read(self.prepare_path(path)).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                OpenReadError::FileDoesNotExist(path.to_owned())
            } else {
                OpenReadError::wrap_io_error(err, path.to_owned())
            }
        })
    }

    fn atomic_write(&self, path: &Path, data: &[u8]) -> std::io::Result<()> {
        let path = self.prepare_path(path);
        std::fs::write(&path, data)?;

        Ok(())
    }

    fn sync_directory(&self) -> std::io::Result<()> {
        Ok(())
    }

    fn watch(&self, watch_callback: WatchCallback) -> crate::Result<WatchHandle> {
        Ok(self.watcher.watch(watch_callback))
    }
}

#[allow(unused)]
pub(crate) fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    std::fs::File::open(path)?.write_all(content)
}