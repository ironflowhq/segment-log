use super::*;
use survey::ReadOnly;
mod log_filter;
pub(crate) use log_filter::Mode as LogFilterMode;
use log_filter::{Builder, LogFilter};

pub(super) struct Keys {
    hash: KeyHash,
    active: BTreeMap<Vec<u8>, ActiveKey>,
    sealed: Vec<Arc<SealedKeys>>,
    log: Option<Arc<LogFilter>>,
    builder: Option<Builder>,
}

/// A staged key's position is the current group's frame position, which no durable frame has.
#[derive(Clone, Copy, Debug)]
pub(super) struct ActiveKey {
    pub position: u32,
    pub record: u32,
}

pub(super) struct SealedKeys {
    base: u64,
    next: u64,
    filter_at: u64,
    pages_at: u64,
    filter: codec::KeyFilter,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct SealedSummary {
    pub base: u64,
    pub next: u64,
    pub filter_at: u64,
    pub footer_at: u64,
}

pub(super) struct KeyHash {
    log_id: codec::LogId,
    #[cfg(test)]
    forced: BTreeMap<Vec<u8>, u64>,
}

pub(super) struct Staged<'a> {
    pub position: u64,
    pub base: u64,
    pub requests: &'a [Request],
    /// Request and record index of each staged record, by offset past `base`.
    pub origins: &'a [(usize, usize)],
    pub assigned: &'a [Revision],
}

pub(super) enum Classification {
    New,
    DurableRetry(Vec<Position>),
    PendingRetry(Vec<Position>),
    Rejected(CommitError),
    LookupFailure,
}

struct LookupFailure;

impl From<io::Error> for LookupFailure {
    fn from(_: io::Error) -> Self {
        Self
    }
}

impl From<codec::Error> for LookupFailure {
    fn from(_: codec::Error) -> Self {
        Self
    }
}

struct Original {
    position: Position,
    pending: bool,
    same: bool,
}

impl KeyHash {
    pub(super) fn new(log_id: codec::LogId) -> Self {
        Self {
            log_id,
            #[cfg(test)]
            forced: BTreeMap::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn forcing(self, forced: BTreeMap<Vec<u8>, u64>) -> Self {
        Self { forced, ..self }
    }

    /// The key index entries of a segment's keyed records, as a roll writes them.
    pub(super) fn entries(&self, keys: &BTreeMap<Vec<u8>, ActiveKey>) -> Vec<codec::KeyEntry> {
        keys.iter()
            .map(|(key, at)| codec::KeyEntry {
                hash: self.of(key),
                position: at.position,
                record: at.record,
            })
            .collect()
    }

    fn of(&self, key: &[u8]) -> u64 {
        #[cfg(test)]
        if let Some(&hash) = self.forced.get(key) {
            return hash;
        }
        self.log_id.key_hash(key)
    }
}

impl SealedKeys {
    /// Runs are consecutive and joined in order, so the first failure in segment order is the
    /// error that a load in one thread would give.
    pub(super) fn load_all(
        storage: &ReadOnly<'_>,
        dir: &Path,
        log_id: &codec::LogId,
        summaries: &[SealedSummary],
    ) -> Result<Vec<Self>, OpenError> {
        let load_run = |run: &[SealedSummary]| {
            run.iter()
                .map(|&summary| Self::load(storage, dir, log_id, summary))
                .collect::<Result<Vec<_>, _>>()
        };
        let threads = thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(summaries.len());
        if threads <= 1 {
            return load_run(summaries);
        }
        thread::scope(|scope| {
            let loads: Vec<_> = summaries
                .chunks(summaries.len().div_ceil(threads))
                .map(|run| {
                    thread::Builder::new()
                        .name("segment-log-open".into())
                        .spawn_scoped(scope, move || load_run(run))
                })
                .collect();
            let finished: Vec<_> = loads
                .into_iter()
                .map(|load| load.map(|handle| handle.join()))
                .collect();
            let mut keys = Vec::with_capacity(summaries.len());
            for run in finished {
                match run? {
                    Ok(run) => keys.extend(run?),
                    Err(panic) => std::panic::resume_unwind(panic),
                }
            }
            Ok(keys)
        })
    }

    fn load(
        storage: &ReadOnly<'_>,
        dir: &Path,
        log_id: &codec::LogId,
        summary: SealedSummary,
    ) -> Result<Self, OpenError> {
        let corrupt = || OpenError::Corrupt {
            segment: summary.base,
            position: summary.filter_at,
        };
        let path = segment_path(dir, summary.base);
        let bytes = storage
            .frame(
                &path,
                Purpose::KeyFilter,
                summary.filter_at,
                summary.footer_at,
            )?
            .ok_or_else(corrupt)?;
        let keys = Self::decode(summary, log_id, &bytes).ok_or_else(corrupt)?;
        let pages = (keys.filter.count() as usize).div_ceil(codec::PAGE_ENTRIES);
        if keys.pages_at + (codec::PAGE_SIZE * pages) as u64 != summary.footer_at {
            return Err(corrupt());
        }
        Ok(keys)
    }

    pub(super) fn written(summary: SealedSummary, log_id: &codec::LogId, filter: &[u8]) -> Self {
        Self::decode(summary, log_id, filter).expect("the writer encodes a valid key filter")
    }

    fn decode(summary: SealedSummary, log_id: &codec::LogId, bytes: &[u8]) -> Option<Self> {
        let frame = codec::decode(log_id, bytes).ok()?;
        let codec::Body::KeyFilter(filter) = frame.body else {
            return None;
        };
        (frame.base_offset == summary.next).then(|| Self {
            base: summary.base,
            next: summary.next,
            filter_at: summary.filter_at,
            pages_at: summary.filter_at + (bytes.len() + 8 + codec::BODY_HEADER_LEN) as u64,
            filter,
        })
    }

    fn find(
        &self,
        storage: &dyn Storage,
        dir: &Path,
        log_id: &codec::LogId,
        hash: u64,
        incoming: &NewRecord,
    ) -> Result<Option<Original>, LookupFailure> {
        let path = segment_path(dir, self.base);
        for page in self.filter.index_pages(hash) {
            let at = self.pages_at + (codec::PAGE_SIZE * page) as u64;
            let bytes = read_exact(storage, &path, Purpose::KeyIndex, at, codec::PAGE_SIZE)?;
            let entries =
                codec::decode_page(&bytes, page, self.filter.count(), self.filter.fence(page))?;
            for entry in entries.iter().filter(|entry| entry.hash == hash) {
                let position = u64::from(entry.position);
                if position < codec::HEADER_LEN as u64 {
                    return Err(LookupFailure);
                }
                let frame = ReadOnly::new(storage)
                    .frame(&path, Purpose::Lookup, position, self.filter_at)?
                    .ok_or(LookupFailure)?;
                let group = codec::decode(log_id, &frame)?
                    .into_group()
                    .filter(|group| {
                        group.base_offset >= self.base
                            && group.base_offset + group.entries.len() as u64 <= self.next
                    })
                    .ok_or(LookupFailure)?;
                if let Some(original) = compare(&group, entry.record, incoming)? {
                    return Ok(Some(original));
                }
            }
        }
        Ok(None)
    }
}

fn compare(
    group: &codec::Group<'_>,
    record: u32,
    incoming: &NewRecord,
) -> Result<Option<Original>, LookupFailure> {
    let entry = group.entries.get(record as usize).ok_or(LookupFailure)?;
    if entry.key != incoming.key.as_deref() {
        return Ok(None);
    }
    Ok(Some(Original {
        position: Position {
            offset: group.base_offset + u64::from(record),
            revision: entry.revision,
        },
        pending: false,
        same: entry.stream == incoming.stream && entry.payload == incoming.payload,
    }))
}

impl Keys {
    pub(super) fn new(
        hash: KeyHash,
        active: BTreeMap<Vec<u8>, ActiveKey>,
        sealed: Vec<SealedKeys>,
    ) -> Self {
        Self {
            hash,
            active,
            sealed: sealed.into_iter().map(Arc::new).collect(),
            log: None,
            builder: None,
        }
    }

    /// Starts the log key filter's build over the sealed segments known now (ADR 0051).
    pub(super) fn start_building(&mut self, view: &View, mode: LogFilterMode) {
        self.builder = Builder::start(
            mode,
            view.storage.clone(),
            view.dir.clone(),
            self.sealed.clone(),
        );
        self.refresh();
    }

    /// Takes the builder's newest filter. Any filter it hands over gives the same answers, so the
    /// writer takes it whenever it is between lookups.
    pub(super) fn refresh(&mut self) {
        if let Some(filter) = self.builder.as_mut().and_then(Builder::ready) {
            self.log = Some(filter);
        }
    }

    /// A poisoned writer looks up no key again, so its builder has nothing left to serve.
    pub(super) fn stop_building(&mut self) {
        self.builder = None;
    }

    pub(super) fn stage(&mut self, submit: &Submit, position: u64, first: usize) {
        for (i, record) in submit.records.iter().enumerate() {
            if let Some(key) = &record.key {
                self.active.insert(
                    key.clone(),
                    ActiveKey {
                        position: position as u32,
                        record: (first + i) as u32,
                    },
                );
            }
        }
    }

    pub(super) fn keyed(&self) -> usize {
        self.active.len()
    }

    pub(super) fn index_entries(&self) -> Vec<codec::KeyEntry> {
        self.hash.entries(&self.active)
    }

    pub(super) fn roll(&mut self, sealed: SealedKeys) {
        let sealed = Arc::new(sealed);
        if let Some(builder) = &mut self.builder {
            builder.rolled(sealed.clone());
        }
        self.sealed.push(sealed);
        self.active.clear();
        self.refresh();
    }

    #[cfg(test)]
    pub(super) fn memory(&self) -> KeyMemory {
        KeyMemory {
            filter_bytes: self.sealed.iter().map(|s| s.filter.memory_bytes()).sum(),
            sealed_keys: self
                .sealed
                .iter()
                .map(|s| u64::from(s.filter.count()))
                .sum(),
            active_keys: self.active.len(),
            log_filter_bytes: self.log.as_ref().map_or(0, |log| log.memory_bytes()),
            log_filter_covered: self.log.as_ref().map_or(0, |log| log.covered()),
        }
    }

    pub(super) fn classify(
        &self,
        view: &View,
        submit: &Submit,
        staged: &Staged<'_>,
    ) -> Classification {
        let mut positions = Vec::new();
        let mut has_pending = false;
        for (index, record) in submit.records.iter().enumerate() {
            let original = match self.find(view, record, staged) {
                Ok(Some(original)) => original,
                Ok(None) => continue,
                Err(LookupFailure) => return Classification::LookupFailure,
            };
            if !original.same {
                return Classification::Rejected(CommitError::KeyConflict { index });
            }
            has_pending |= original.pending;
            if positions.is_empty() {
                positions = Vec::with_capacity(submit.records.len());
            }
            positions.push(original.position);
        }
        if positions.is_empty() {
            Classification::New
        } else if positions.len() == submit.records.len() {
            if has_pending {
                Classification::PendingRetry(positions)
            } else {
                Classification::DurableRetry(positions)
            }
        } else {
            Classification::Rejected(CommitError::PartialRetry)
        }
    }

    fn find(
        &self,
        view: &View,
        incoming: &NewRecord,
        staged: &Staged<'_>,
    ) -> Result<Option<Original>, LookupFailure> {
        let Some(key) = &incoming.key else {
            return Ok(None);
        };
        if let Some(&ActiveKey { position, record }) = self.active.get(key) {
            if u64::from(position) == staged.position {
                let (request, index) = staged.origins[record as usize];
                let entry = &staged.requests[request].submit.records[index];
                return Ok(Some(Original {
                    position: Position {
                        offset: staged.base + u64::from(record),
                        revision: staged.assigned[record as usize],
                    },
                    pending: true,
                    same: entry.stream == incoming.stream && entry.payload == incoming.payload,
                }));
            }
            // The exact map names this key's record, so another key there is a damaged file.
            return active_record(view, position, record, incoming)?
                .map(Some)
                .ok_or(LookupFailure);
        }
        if self.sealed.is_empty() {
            return Ok(None);
        }
        let hash = self.hash.of(key);
        // The log filter answers for the oldest segments it covers. A "no" there skips their
        // segment filters, which a Bloom filter's lack of false negatives makes safe (ADR 0051).
        let covered = self.log.as_ref().map_or(0, |log| log.covered());
        let skip = if covered > 0 && !self.log.as_ref().unwrap().may_contain(hash) {
            covered
        } else {
            0
        };
        let probed = &self.sealed[skip..];
        // Each probe below misses the caches, because the filters together are far larger, and
        // its early exit would take those misses one after another. Loading every filter's line
        // first lets the misses overlap. `black_box` keeps the loads, whose values nothing uses.
        std::hint::black_box(
            probed
                .iter()
                .fold(0, |lines, segment| lines ^ segment.filter.touch(hash)),
        );
        for segment in probed.iter().rev() {
            if segment.filter.may_contain(hash)
                && let Some(original) = segment.find(
                    view.storage.as_ref(),
                    &view.dir,
                    &view.log_id,
                    hash,
                    incoming,
                )?
            {
                return Ok(Some(original));
            }
        }
        Ok(None)
    }
}

fn active_record(
    view: &View,
    position: u32,
    record: u32,
    incoming: &NewRecord,
) -> Result<Option<Original>, LookupFailure> {
    let (base, span) = {
        let published = view.published.lock().unwrap();
        let spans = &published.newest.spans;
        let at = spans
            .binary_search_by_key(&u64::from(position), |span| span.position)
            .map_err(|_| LookupFailure)?;
        (published.newest.base, spans[at])
    };
    let bytes = read_exact(
        view.storage.as_ref(),
        &segment_path(&view.dir, base),
        Purpose::Lookup,
        span.position,
        span.length as usize,
    )?;
    let group = codec::decode(&view.log_id, &bytes)?
        .into_group()
        .filter(|group| {
            group.base_offset == span.base && group.entries.len() == span.count as usize
        })
        .ok_or(LookupFailure)?;
    compare(&group, record, incoming)
}
