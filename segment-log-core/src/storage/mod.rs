use std::{
    io,
    path::{Path, PathBuf},
};
pub(crate) mod real;

#[cfg(any(test, fuzzing))]
pub(crate) mod sim;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Purpose {
    Open,
    Group,
    Recovery,
    Lookup,
    Roll,
    Summary,
    KeyFilter,
    KeyIndex,
    Seal,
    /// The log key filter's builder reads key index frames after open (ADR 0051).
    LogFilter,
    /// The operator repair tool's backup and truncation (ADR 0065).
    Repair,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    CreateDir,
    Create,
    Open,
    Append(Vec<u8>),
    Write(u64, Vec<u8>),
    Read(u64, usize),
    SyncData,
    SyncAll,
    SyncDir,
    Truncate(u64),
    Remove,
    List,
    Random(usize),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Call {
    pub path: PathBuf,
    pub purpose: Purpose,
    pub op: Op,
}
impl Call {
    pub(crate) fn new(path: impl AsRef<Path>, purpose: Purpose, op: Op) -> Self {
        Self {
            path: path.as_ref().to_owned(),
            purpose,
            op,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Value {
    Unit,
    Length(u64),
    Bytes(Vec<u8>),
    Names(Vec<PathBuf>),
}
pub(crate) trait Lock: Send {}
// One command seam keeps recorded calls identical to the operations being simulated.
pub(crate) trait Storage: Send + Sync {
    fn call(&self, call: Call) -> io::Result<Value>;
    fn lock(&self, path: &Path) -> io::Result<Box<dyn Lock>>;
    /// Wall-clock milliseconds since the Unix epoch, for a group's append time. It sits on this
    /// seam so that the simulation and the fixtures can fix it. A clock before 1970 reads 0.
    fn now_millis(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }
}
