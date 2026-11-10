use super::*;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::FileExt,
    sync::{Arc, Mutex},
};
#[derive(Default)]
pub(crate) struct Real {
    written: Mutex<Option<(PathBuf, Arc<File>)>>,
}
impl Real {
    // Only the file being changed keeps a handle, so open handles do not grow with the
    // segment count. A failed flush must surface on the handle that wrote the bytes
    // (ADR 0004), and the writer changes another file only after this one's last flush
    // succeeded. The lock covers only the handle lookup, so a read never waits for a flush.
    // Reads are positioned and leave the file cursor alone. Only one thread at a time
    // moves it: `open` during recovery, then the writer thread.
    fn with_written<T>(
        &self,
        path: &Path,
        f: impl FnOnce(&File) -> io::Result<T>,
    ) -> io::Result<T> {
        let file = {
            let mut written = self.written.lock().unwrap();
            match &*written {
                Some((cached, file)) if cached == path => file.clone(),
                _ => {
                    let file = Arc::new(OpenOptions::new().read(true).write(true).open(path)?);
                    *written = Some((path.to_owned(), file.clone()));
                    file
                }
            }
        };
        f(&file)
    }
    fn with_readable<T>(
        &self,
        path: &Path,
        f: impl FnOnce(&File) -> io::Result<T>,
    ) -> io::Result<T> {
        let cached = match &*self.written.lock().unwrap() {
            Some((cached, file)) if cached == path => Some(file.clone()),
            _ => None,
        };
        match cached {
            Some(file) => f(&file),
            None => f(&File::open(path)?),
        }
    }
}
struct Guard(File);
impl Lock for Guard {}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
impl Storage for Real {
    fn call(&self, call: Call) -> io::Result<Value> {
        let path = call.path;
        match call.op {
            Op::CreateDir => fs::create_dir(path)?,
            Op::Create => {
                let file = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true)
                    .open(&path)?;
                *self.written.lock().unwrap() = Some((path, Arc::new(file)));
            }
            Op::Open => {
                return self.with_readable(&path, |file| Ok(Value::Length(file.metadata()?.len())));
            }
            Op::Append(bytes) => self.with_written(&path, |mut file| {
                file.seek(SeekFrom::End(0))?;
                file.write_all(&bytes)
            })?,
            Op::Write(pos, bytes) => self.with_written(&path, |mut file| {
                file.seek(SeekFrom::Start(pos))?;
                file.write_all(&bytes)
            })?,
            Op::Read(pos, len) => {
                return self.with_readable(&path, |file| {
                    let mut bytes = vec![0; len];
                    let mut used = 0;
                    while used < len {
                        let n = file.read_at(&mut bytes[used..], pos + used as u64)?;
                        if n == 0 {
                            break;
                        }
                        used += n;
                    }
                    bytes.truncate(used);
                    Ok(Value::Bytes(bytes))
                });
            }
            Op::SyncData => self.with_written(&path, |file| file.sync_data())?,
            Op::SyncAll => self.with_written(&path, |file| file.sync_all())?,
            Op::SyncDir => File::open(path)?.sync_all()?,
            Op::Truncate(len) => self.with_written(&path, |file| file.set_len(len))?,
            Op::Remove => {
                fs::remove_file(&path)?;
                let mut written = self.written.lock().unwrap();
                if written.as_ref().is_some_and(|(cached, _)| *cached == path) {
                    *written = None;
                }
            }
            Op::List => {
                let mut names = fs::read_dir(path)?
                    .map(|entry| entry.map(|e| PathBuf::from(e.file_name())))
                    .collect::<io::Result<Vec<_>>>()?;
                names.sort();
                return Ok(Value::Names(names));
            }
            Op::Random(len) => {
                let mut bytes = vec![0; len];
                File::open("/dev/urandom")?.read_exact(&mut bytes)?;
                return Ok(Value::Bytes(bytes));
            }
        }
        Ok(Value::Unit)
    }
    fn lock(&self, path: &Path) -> io::Result<Box<dyn Lock>> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        file.try_lock().map_err(|e| match e {
            std::fs::TryLockError::WouldBlock => io::Error::from(io::ErrorKind::WouldBlock),
            std::fs::TryLockError::Error(e) => e,
        })?;
        Ok(Box::new(Guard(file)))
    }
}
