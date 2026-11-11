use super::*;
use survey::{Ending, ReadOnly};

/// Takes no lock and changes no file, so that it works on a read-only volume (ADR 0016).
pub struct LogReader {
    view: View,
}

impl LogReader {
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, OpenError> {
        Self::with_storage(dir.as_ref(), Arc::new(storage::real::Real::default()))
    }

    pub(crate) fn with_storage(dir: &Path, storage: Arc<dyn Storage>) -> Result<Self, OpenError> {
        let reader = ReadOnly::new(storage.as_ref());
        let bases = survey::segment_bases(&reader, dir)?;
        // A seal is the last write to a log, so a torn header-only file means a roll that crashed.
        let Some(&newest) = bases.last() else {
            return Err(OpenError::NotSealed);
        };
        if survey::is_torn_header(&reader, dir, newest)? {
            return Err(OpenError::NotSealed);
        }
        let found = survey::survey(&reader, dir, &bases)?;
        let Ending::Sealed(_) = found.newest.ending else {
            return Err(OpenError::NotSealed);
        };
        Ok(Self {
            view: View {
                storage,
                dir: dir.to_owned(),
                log_id: found.log_id,
                published: Mutex::new(found.published),
            },
        })
    }

    pub fn watermark(&self) -> LogOffset {
        self.view.watermark()
    }

    pub fn read(&self, from: LogOffset, max: usize) -> Result<Vec<Record>, ReadError> {
        self.view.read(from, max)
    }

    pub fn stream_head(&self, stream: &[u8]) -> Option<StreamHead> {
        self.view.stream_head(stream)
    }
}
