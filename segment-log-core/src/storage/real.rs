use super::*;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    sync::Mutex,
};
#[derive(Default)]
pub(crate) struct Real {
    files: Mutex<BTreeMap<PathBuf, File>>,
}
impl Real {
    fn with_file<T>(
        &self,
        path: &Path,
        f: impl FnOnce(&mut File) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut files = self.files.lock().unwrap();
        if !files.contains_key(path) {
            files.insert(
                path.to_owned(),
                OpenOptions::new().read(true).write(true).open(path)?,
            );
        }
        f(files.get_mut(path).unwrap())
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
                self.files.lock().unwrap().insert(path, file);
            }
            Op::Open => {
                return self.with_file(&path, |file| Ok(Value::Length(file.metadata()?.len())));
            }
            Op::Append(bytes) => self.with_file(&path, |file| {
                file.seek(SeekFrom::End(0))?;
                file.write_all(&bytes)
            })?,
            Op::Write(pos, bytes) => self.with_file(&path, |file| {
                file.seek(SeekFrom::Start(pos))?;
                file.write_all(&bytes)
            })?,
            Op::Read(pos, len) => {
                return self.with_file(&path, |file| {
                    file.seek(SeekFrom::Start(pos))?;
                    let mut bytes = vec![0; len];
                    let mut used = 0;
                    while used < len {
                        let n = file.read(&mut bytes[used..])?;
                        if n == 0 {
                            break;
                        }
                        used += n;
                    }
                    bytes.truncate(used);
                    Ok(Value::Bytes(bytes))
                });
            }
            Op::SyncData => self.with_file(&path, |file| file.sync_data())?,
            Op::SyncAll => self.with_file(&path, |file| file.sync_all())?,
            Op::SyncDir => File::open(path)?.sync_all()?,
            Op::Truncate(len) => self.with_file(&path, |file| file.set_len(len))?,
            Op::Remove => {
                fs::remove_file(&path)?;
                self.files.lock().unwrap().remove(&path);
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
