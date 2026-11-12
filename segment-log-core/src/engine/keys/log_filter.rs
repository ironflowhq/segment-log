//! The log key filter: one blocked Bloom filter in memory over the keys of the oldest sealed
//! segments, in front of their segment filters (ADR 0051). It is a cache. It never reaches a
//! file, and losing it costs speed, never an answer.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

/// 8 bits for each key when the filter is full (ADR 0051 rule 4).
const KEYS_PER_BLOCK: u64 = 64;
/// Pages that one builder read takes, so that a stop waits for at most one read.
const CHUNK_PAGES: usize = 64;

/// One cache line, so that a probe reads one line, as in a segment filter.
#[repr(align(64))]
#[derive(Default)]
struct Block([AtomicU64; 8]);

pub(in crate::engine) struct LogFilter {
    blocks: Box<[Block]>,
    /// How many sealed segments, from the oldest, have every key in the filter. The builder
    /// stores it after the last bit of a segment, so a writer that loads it sees those bits.
    covered: AtomicUsize,
}

impl LogFilter {
    /// Room for twice `keys`, so that a new filter holds 16 bits for each key it starts with.
    fn sized_for(keys: u64) -> Self {
        let blocks = (2 * keys).div_ceil(KEYS_PER_BLOCK).max(1) as usize;
        Self {
            blocks: (0..blocks).map(|_| Block::default()).collect(),
            covered: AtomicUsize::new(0),
        }
    }

    fn capacity(&self) -> u64 {
        self.blocks.len() as u64 * KEYS_PER_BLOCK
    }

    fn insert(&self, hash: u64) {
        let (block, bits) = codec::filter_probes(hash, self.blocks.len());
        let words = &self.blocks[block].0;
        for bit in bits {
            words[bit / 64].fetch_or(1 << (bit % 64), Ordering::Relaxed);
        }
    }

    pub(super) fn may_contain(&self, hash: u64) -> bool {
        let (block, bits) = codec::filter_probes(hash, self.blocks.len());
        let words = &self.blocks[block].0;
        bits.into_iter()
            .all(|bit| words[bit / 64].load(Ordering::Relaxed) & (1 << (bit % 64)) != 0)
    }

    pub(super) fn covered(&self) -> usize {
        self.covered.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(super) fn memory_bytes(&self) -> usize {
        size_of_val(&*self.blocks)
    }
}

/// Where the build runs. Tests run it on the writer thread, so that simulated traces stay the
/// same from run to run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Mode {
    Background,
    #[cfg_attr(not(test), allow(dead_code))]
    #[default]
    Inline,
    #[cfg_attr(not(test), allow(dead_code))]
    Off,
}

/// The builder's state. It adds sealed segments to `building` in order, and gives the writer a
/// filter once that filter covers at least what the writer's current filter covers.
struct Build {
    storage: Arc<dyn Storage>,
    dir: PathBuf,
    segments: Vec<Arc<SealedKeys>>,
    building: Option<Arc<LogFilter>>,
    published: Option<Arc<LogFilter>>,
    inserted: u64,
    stopped: bool,
}

impl Build {
    fn catch_up(&mut self, stop: &AtomicBool, publish: &mut dyn FnMut(Arc<LogFilter>)) {
        while !self.stopped {
            let covered = self.building.as_ref().map_or(0, |filter| filter.covered());
            if let Some(building) = &self.building
                && !self
                    .published
                    .as_ref()
                    .is_some_and(|published| Arc::ptr_eq(published, building))
                && covered >= self.published.as_ref().map_or(0, |p| p.covered())
            {
                self.published = Some(building.clone());
                publish(building.clone());
            }
            let Some(next) = self.segments.get(covered).cloned() else {
                return;
            };
            let count = u64::from(next.filter.count());
            if self
                .building
                .as_ref()
                .is_none_or(|filter| self.inserted + count > filter.capacity())
            {
                let keys = self.segments.iter().map(|s| u64::from(s.filter.count()));
                self.building = Some(Arc::new(LogFilter::sized_for(keys.sum())));
                self.inserted = 0;
                continue;
            }
            let building = self.building.clone().unwrap();
            if self.add(&building, &next, stop).is_err() {
                self.stopped = true;
                return;
            }
            self.inserted += count;
            building.covered.store(covered + 1, Ordering::Release);
        }
    }

    /// The key index pages carry their own CRCs, so a run of them is checked page by page.
    fn add(&self, filter: &LogFilter, segment: &SealedKeys, stop: &AtomicBool) -> Result<(), ()> {
        let path = segment_path(&self.dir, segment.base);
        let count = segment.filter.count();
        let pages = codec::page_count(count);
        for first in (0..pages).step_by(CHUNK_PAGES) {
            if stop.load(Ordering::Relaxed) {
                return Err(());
            }
            let n = CHUNK_PAGES.min(pages - first);
            let bytes = read_exact(
                self.storage.as_ref(),
                &path,
                Purpose::LogFilter,
                segment.pages_at + (codec::PAGE_SIZE * first) as u64,
                codec::PAGE_SIZE * n,
            )
            .map_err(drop)?;
            for (i, page) in bytes.chunks(codec::PAGE_SIZE).enumerate() {
                let number = first + i;
                let entries = codec::decode_page(page, number, count, segment.filter.fence(number))
                    .map_err(drop)?;
                for entry in entries {
                    filter.insert(entry.hash);
                }
            }
        }
        Ok(())
    }
}

/// The writer's end of the build. Dropping it stops the builder and waits for it.
pub(super) struct Builder {
    stop: Arc<AtomicBool>,
    run: Run,
}

enum Run {
    Background {
        rolls: Option<mpsc::Sender<Arc<SealedKeys>>>,
        filters: mpsc::Receiver<Arc<LogFilter>>,
        thread: Option<JoinHandle<()>>,
    },
    Inline {
        build: Box<Build>,
        ready: Option<Arc<LogFilter>>,
    },
}

impl Builder {
    pub(super) fn start(
        mode: Mode,
        storage: Arc<dyn Storage>,
        dir: PathBuf,
        segments: Vec<Arc<SealedKeys>>,
    ) -> Option<Self> {
        let mut build = Box::new(Build {
            storage,
            dir,
            segments,
            building: None,
            published: None,
            inserted: 0,
            stopped: false,
        });
        let stop = Arc::new(AtomicBool::new(false));
        let run = match mode {
            Mode::Off => return None,
            Mode::Inline => {
                let mut ready = None;
                build.catch_up(&stop, &mut |filter| ready = Some(filter));
                Run::Inline { build, ready }
            }
            Mode::Background => {
                let (rolls, rolled) = mpsc::channel::<Arc<SealedKeys>>();
                let (send, filters) = mpsc::channel();
                let thread_stop = stop.clone();
                // Without a thread the writer probes every segment filter, as without this cache.
                let thread = thread::Builder::new()
                    .name("segment-log-key-filter".into())
                    .spawn(move || {
                        let mut publish = |filter| {
                            let _ = send.send(filter);
                        };
                        build.catch_up(&thread_stop, &mut publish);
                        while let Ok(segment) = rolled.recv() {
                            build.segments.push(segment);
                            build.segments.extend(rolled.try_iter());
                            build.catch_up(&thread_stop, &mut publish);
                        }
                    })
                    .ok();
                Run::Background {
                    rolls: Some(rolls),
                    filters,
                    thread,
                }
            }
        };
        Some(Self { stop, run })
    }

    pub(super) fn rolled(&mut self, segment: Arc<SealedKeys>) {
        match &mut self.run {
            Run::Background { rolls, .. } => {
                if let Some(rolls) = rolls {
                    let _ = rolls.send(segment);
                }
            }
            Run::Inline { build, ready } => {
                build.segments.push(segment);
                build.catch_up(&self.stop, &mut |filter| *ready = Some(filter));
            }
        }
    }

    /// The newest filter that the builder has handed over since the last call.
    pub(super) fn ready(&mut self) -> Option<Arc<LogFilter>> {
        match &mut self.run {
            Run::Background { filters, .. } => filters.try_iter().last(),
            Run::Inline { ready, .. } => ready.take(),
        }
    }
}

impl Drop for Builder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Run::Background { rolls, thread, .. } = &mut self.run {
            rolls.take();
            if let Some(thread) = thread.take() {
                let _ = thread.join();
            }
        }
    }
}
