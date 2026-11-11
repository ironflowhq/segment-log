use std::{
    io,
    path::{Path, PathBuf},
};
pub(crate) mod real;

#[cfg(test)]
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
    /// Open's reads of a locator and the footer it names.
    Summary,
    /// Open's reads of a sealed segment's key filter frame.
    KeyFilter,
    /// A writer key lookup's reads of a sealed segment's key index pages.
    KeyIndex,
}
impl Purpose {
    #[cfg(test)]
    pub(crate) const ALL: [Purpose; 8] = [
        Purpose::Open,
        Purpose::Group,
        Purpose::Recovery,
        Purpose::Lookup,
        Purpose::Roll,
        Purpose::Summary,
        Purpose::KeyFilter,
        Purpose::KeyIndex,
    ];
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
}
