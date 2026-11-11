// Throwaway M4 key-lookup prototype. Not production code.
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::hash::{BuildHasherDefault, Hasher};
use std::hint::black_box;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

// ---------- allocation accounting (measurement only) ----------
struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Relaxed);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            LIVE.fetch_add(l.size(), Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            LIVE.fetch_sub(l.size(), Relaxed);
            LIVE.fetch_add(n, Relaxed);
        }
        q
    }
}
#[global_allocator]
static GA: Counting = Counting;

// ---------- cold reads via F_NOCACHE (measurement only) ----------
static NOCACHE: AtomicBool = AtomicBool::new(false);
unsafe extern "C" {
    fn fcntl(fd: i32, cmd: i32, ...) -> i32;
}
const F_NOCACHE: i32 = 48;
fn set_nocache(f: &File) {
    let r = unsafe { fcntl(f.as_raw_fd(), F_NOCACHE, 1i32) };
    assert_eq!(r, 0, "F_NOCACHE failed");
}
fn open_read(p: &Path) -> File {
    let f = File::open(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    if NOCACHE.load(Relaxed) {
        set_nocache(&f);
    }
    f
}
fn write_file(p: &Path, bytes: &[u8], sync: bool, nocache: bool) {
    let mut f = OpenOptions::new().create(true).write(true).truncate(true).open(p).unwrap();
    if nocache {
        set_nocache(&f);
    }
    f.write_all(bytes).unwrap();
    if sync {
        f.sync_all().unwrap();
    }
}

// ---------- CRC-32C, slicing-by-8, same scheme as segment-log-core codec ----------
const fn make_tables() -> [[u32; 256]; 8] {
    let mut t = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut j = 0;
        while j < 8 {
            c = if c & 1 != 0 { (c >> 1) ^ 0x82F6_3B78 } else { c >> 1 };
            j += 1;
        }
        t[0][i] = c;
        i += 1;
    }
    let mut i = 0;
    while i < 256 {
        let mut k = 1;
        while k < 8 {
            t[k][i] = (t[k - 1][i] >> 8) ^ t[0][(t[k - 1][i] & 0xff) as usize];
            k += 1;
        }
        i += 1;
    }
    t
}
static TABLES: [[u32; 256]; 8] = make_tables();
fn crc32c(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    let (chunks, rest) = bytes.as_chunks::<8>();
    for chunk in chunks {
        let w = u64::from_le_bytes(*chunk) ^ u64::from(crc);
        crc = TABLES[7][(w & 0xff) as usize]
            ^ TABLES[6][((w >> 8) & 0xff) as usize]
            ^ TABLES[5][((w >> 16) & 0xff) as usize]
            ^ TABLES[4][((w >> 24) & 0xff) as usize]
            ^ TABLES[3][((w >> 32) & 0xff) as usize]
            ^ TABLES[2][((w >> 40) & 0xff) as usize]
            ^ TABLES[1][((w >> 48) & 0xff) as usize]
            ^ TABLES[0][(w >> 56) as usize];
    }
    for &b in rest {
        crc = (crc >> 8) ^ TABLES[0][((crc ^ u32::from(b)) & 0xff) as usize];
    }
    !crc
}

// ---------- 64-bit key hash: inline wyhash-style multiply-fold, fixed seed ----------
const SEED: u64 = 0x5eed_1234_abcd_0001;
const P0: u64 = 0xa076_1d64_78bd_642f;
const P1: u64 = 0xe703_7ed1_a0b4_28db;
const P2: u64 = 0x8ebc_6af0_9c88_c6e3;
#[inline]
fn mum(a: u64, b: u64) -> u64 {
    let r = (a as u128) * (b as u128);
    (r as u64) ^ ((r >> 64) as u64)
}
#[inline]
fn h64(key: &[u8]) -> u64 {
    let mut h = SEED ^ P0 ^ (key.len() as u64).wrapping_mul(P1);
    let (chunks, rest) = key.as_chunks::<8>();
    for c in chunks {
        h = mum(u64::from_le_bytes(*c) ^ P1, h ^ P2);
    }
    if !rest.is_empty() {
        let mut t = [0u8; 8];
        t[..rest.len()].copy_from_slice(rest);
        h = mum(u64::from_le_bytes(t) ^ P1, h ^ P2);
    }
    mum(h ^ P0, key.len() as u64 ^ P1)
}
fn sip13(key: &[u8]) -> u64 {
    #[allow(deprecated)]
    let mut s = std::hash::SipHasher::new_with_keys(SEED, P0);
    s.write(key);
    s.finish()
}

// ---------- synthetic keys ----------
fn splitmix(x: &mut u64) -> u64 {
    *x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
fn key(seg: u32, i: u32, out: &mut Vec<u8>) {
    let mut s = 0x1234_5678_9abc_def0 ^ ((seg as u64) << 40) ^ i as u64;
    splitmix(&mut s);
    let len = 16 + (splitmix(&mut s) % 21) as usize;
    out.clear();
    while out.len() < len {
        let w = splitmix(&mut s).to_le_bytes();
        let take = (len - out.len()).min(8);
        out.extend_from_slice(&w[..take]);
    }
}
const NEG_SEG: u32 = 0x00ff_0000;

// ---------- blocked Bloom filter, 64-byte blocks ----------
#[derive(Clone, Copy, Default)]
#[repr(C, align(64))]
struct Block([u64; 8]);
const SALT: [u32; 16] = [
    0x47b6_137b, 0x4497_4d91, 0x8824_ad5b, 0xa2b7_289d, 0x7054_95c7, 0x2df1_424b, 0x9efc_4947,
    0x5c6b_fb31, 0x1d0f_3e9b, 0x6ac1_e2f5, 0xb3a8_c64d, 0xd279_5ce1, 0x3c4f_7a29, 0x8f15_e4b7,
    0xe9a2_3c55, 0x27d8_b1c3,
];
struct Bloom {
    blocks: Vec<Block>,
    k: u32,
}
impl Bloom {
    fn new(nblocks: usize, k: u32) -> Self {
        Bloom { blocks: vec![Block::default(); nblocks.max(1)], k }
    }
    fn for_keys(n: usize, bits_per_key: f64, k: u32) -> Self {
        Self::new(((n as f64 * bits_per_key) / 512.0).ceil() as usize, k)
    }
    fn with_bytes(bytes: usize, k: u32) -> Self {
        Self::new(bytes / 64, k)
    }
    #[inline]
    fn idx(&self, h: u64) -> usize {
        (((h >> 32) * self.blocks.len() as u64) >> 32) as usize
    }
    #[inline]
    fn insert(&mut self, h: u64) {
        let i = self.idx(h);
        let b = &mut self.blocks[i].0;
        let lo = h as u32;
        for s in &SALT[..self.k as usize] {
            let bit = lo.wrapping_mul(*s) >> 23;
            b[(bit >> 6) as usize] |= 1u64 << (bit & 63);
        }
    }
    #[inline]
    fn contains(&self, h: u64) -> bool {
        let b = &self.blocks[self.idx(h)].0;
        let lo = h as u32;
        for s in &SALT[..self.k as usize] {
            let bit = lo.wrapping_mul(*s) >> 23;
            if b[(bit >> 6) as usize] & (1u64 << (bit & 63)) == 0 {
                return false;
            }
        }
        true
    }
    fn bytes(&self) -> usize {
        self.blocks.len() * 64
    }
    fn write_to(&self, out: &mut Vec<u8>) {
        for b in &self.blocks {
            for w in b.0 {
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
    }
    fn from_bytes(bytes: &[u8], k: u32) -> Self {
        let mut blocks = Vec::with_capacity(bytes.len() / 64);
        for c in bytes.chunks_exact(64) {
            let mut b = Block::default();
            for (j, w) in c.chunks_exact(8).enumerate() {
                b.0[j] = u64::from_le_bytes(w.try_into().unwrap());
            }
            blocks.push(b);
        }
        Bloom { blocks, k }
    }
}
fn k_opt(bits_per_key: f64) -> u32 {
    ((bits_per_key * std::f64::consts::LN_2).round() as u32).clamp(1, 16)
}

// ---------- per-segment sidecar ----------
// [header 64][fences pc*8][crc4][filter blocks*64][crc4][pad to 4096][pages 4096 each]
// page: 340 entries * (hash u64, rel u32) = 4080 B, count u32 @4080, crc u32 @4092 over 0..4092
static PAGE_SZ: AtomicUsize = AtomicUsize::new(4096);
fn page() -> usize {
    PAGE_SZ.load(Relaxed)
}
fn per_page() -> usize {
    (page() - 16) / 12
}
const PAGE: usize = 4096;
fn bbpk() -> f64 { std::env::var("BPK").ok().and_then(|v| v.parse().ok()).unwrap_or(10.0) }
fn bk() -> u32 { k_opt(bbpk()) }
struct Header {
    count: u32,
    k: u32,
    filter_blocks: u32,
    page_count: u32,
}
impl Header {
    fn fences_off(&self) -> usize {
        64
    }
    fn filter_off(&self) -> usize {
        64 + self.page_count as usize * 8 + 4
    }
    fn pages_off(&self) -> usize {
        (self.filter_off() + self.filter_blocks as usize * 64 + 4).next_multiple_of(4096)
    }
}
fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn le64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
fn parse_header(b: &[u8]) -> Header {
    assert_eq!(&b[0..4], b"SKIX");
    assert_eq!(crc32c(&b[0..60]), le32(b, 60), "header crc");
    Header { count: le32(b, 16), k: le32(b, 20), filter_blocks: le32(b, 24), page_count: le32(b, 28) }
}
fn build_sidecar(seg: u32, entries: &[(u64, u32)], bloom: &Bloom) -> Vec<u8> {
    let pc = entries.len().div_ceil(per_page());
    let pg = page();
    let h = Header { count: entries.len() as u32, k: bloom.k, filter_blocks: bloom.blocks.len() as u32, page_count: pc as u32 };
    let mut out = Vec::with_capacity(h.pages_off() + pc * pg);
    out.extend_from_slice(b"SKIX");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(seg as u64).to_le_bytes());
    for v in [h.count, h.k, h.filter_blocks, h.page_count] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.resize(60, 0);
    let c = crc32c(&out);
    out.extend_from_slice(&c.to_le_bytes());
    let s = out.len();
    for chunk in entries.chunks(per_page()) {
        out.extend_from_slice(&chunk[0].0.to_le_bytes());
    }
    let c = crc32c(&out[s..]);
    out.extend_from_slice(&c.to_le_bytes());
    let s = out.len();
    bloom.write_to(&mut out);
    let c = crc32c(&out[s..]);
    out.extend_from_slice(&c.to_le_bytes());
    out.resize(h.pages_off(), 0);
    for chunk in entries.chunks(per_page()) {
        let s = out.len();
        for (hash, rel) in chunk {
            out.extend_from_slice(&hash.to_le_bytes());
            out.extend_from_slice(&rel.to_le_bytes());
        }
        out.resize(s + pg - 16, 0);
        out.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        out.resize(s + pg - 4, 0);
        let c = crc32c(&out[s..s + pg - 4]);
        out.extend_from_slice(&c.to_le_bytes());
    }
    out
}

struct SegIdx {
    seg: u32,
    file: File,
    fences: Vec<u64>,
    pages_off: u64,
}
#[derive(Default, Clone, Copy)]
struct Stats {
    filter_hits: u64,
    preads: u64,
    fps: u64,
    found: u64,
}
impl SegIdx {
    fn search(&self, h: u64, buf: &mut [u8], st: &mut Stats) -> Option<u32> {
        let p = self.fences.partition_point(|&f| f < h);
        if p > 0 {
            if let Some(r) = self.search_page(p - 1, h, buf, st) {
                return Some(r);
            }
        }
        if p < self.fences.len() && self.fences[p] == h {
            return self.search_page(p, h, buf, st);
        }
        None
    }
    fn search_page(&self, page: usize, h: u64, buf: &mut [u8], st: &mut Stats) -> Option<u32> {
        st.preads += 1;
        let pg = buf.len();
        self.file.read_exact_at(buf, self.pages_off + (page * pg) as u64).unwrap();
        assert_eq!(crc32c(&buf[..pg - 4]), le32(buf, pg - 4), "page crc");
        let n = le32(buf, pg - 16) as usize;
        let (mut lo, mut hi) = (0usize, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if le64(buf, mid * 12) < h {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (lo < n && le64(buf, lo * 12) == h).then(|| le32(buf, lo * 12 + 8))
    }
}

fn seg_path(dir: &Path, s: u32) -> PathBuf {
    dir.join(format!("seg-{s:04}.kidx"))
}
fn global_path(dir: &Path, mib: usize, k: u32) -> PathBuf {
    dir.join(format!("global-{mib}MiB-k{k}.kflt"))
}
fn global_bytes(f: &Bloom, n: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(f.bytes() + 68);
    out.extend_from_slice(b"SKGF");
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&f.k.to_le_bytes());
    out.extend_from_slice(&(f.blocks.len() as u32).to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.resize(60, 0);
    let c = crc32c(&out);
    out.extend_from_slice(&c.to_le_bytes());
    f.write_to(&mut out);
    let c = crc32c(&out[64..]);
    out.extend_from_slice(&c.to_le_bytes());
    out
}

// ---------- scale ----------
#[derive(Clone, Copy)]
struct Scale {
    name: &'static str,
    keys_per_seg: u32,
}
const SEGS: u32 = 100;
const BUDGETS_MIB: [usize; 3] = [4, 16, 64];
fn gk(scale: Scale, mib: usize) -> u32 {
    k_opt((mib * 8 << 20) as f64 / (scale.keys_per_seg as f64 * SEGS as f64))
}

fn segment_entries(seg: u32, n: u32, kb: &mut Vec<u8>) -> Vec<(u64, u32)> {
    let mut e: Vec<(u64, u32)> = (0..n)
        .map(|i| {
            key(seg, i, kb);
            (h64(kb), i)
        })
        .collect();
    e.sort_unstable();
    e
}

fn generate(dir: &Path, scale: Scale, nocache: bool) {
    fs::create_dir_all(dir).unwrap();
    let mut kb = Vec::new();
    let mut globals: Vec<Bloom> = BUDGETS_MIB.iter().map(|&m| Bloom::with_bytes(m << 20, gk(scale, m))).collect();
    for s in 0..SEGS {
        let e = segment_entries(s, scale.keys_per_seg, &mut kb);
        let mut f = Bloom::for_keys(e.len(), bbpk(), bk());
        for &(h, _) in &e {
            f.insert(h);
            for g in &mut globals {
                g.insert(h);
            }
        }
        write_file(&seg_path(dir, s), &build_sidecar(s, &e, &f), false, nocache);
    }
    for (g, &m) in globals.iter().zip(&BUDGETS_MIB) {
        write_file(&global_path(dir, m, g.k), &global_bytes(g, (scale.keys_per_seg * SEGS) as u64), false, nocache);
    }
}

// ---------- candidates ----------
#[derive(Default)]
struct IdHasher(u64);
impl Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _: &[u8]) {
        unreachable!()
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = v;
    }
}
type AMap = HashMap<u64, u64, BuildHasherDefault<IdHasher>>;

fn open_a(dir: &Path) -> AMap {
    let mut files = Vec::new();
    let mut total = 0usize;
    for s in 0..SEGS {
        let f = open_read(&seg_path(dir, s));
        let mut hb = [0u8; 64];
        f.read_exact_at(&mut hb, 0).unwrap();
        let h = parse_header(&hb);
        total += h.count as usize;
        files.push((s, f, h));
    }
    let mut map = AMap::with_capacity_and_hasher(total, Default::default());
    let mut buf = Vec::new();
    for (s, f, h) in files {
        let pg = page();
        buf.resize(h.page_count as usize * pg, 0);
        f.read_exact_at(&mut buf, h.pages_off() as u64).unwrap();
        for page in buf.chunks_exact(pg) {
            assert_eq!(crc32c(&page[..pg - 4]), le32(page, pg - 4), "page crc");
            let n = le32(page, pg - 16) as usize;
            for i in 0..n {
                map.insert(le64(page, i * 12), (s as u64) << 32 | le32(page, i * 12 + 8) as u64);
            }
        }
    }
    map
}

fn open_seg(dir: &Path, s: u32, with_filter: bool) -> (SegIdx, Option<Bloom>) {
    let f = open_read(&seg_path(dir, s));
    let mut hb = [0u8; 64];
    f.read_exact_at(&mut hb, 0).unwrap();
    let h = parse_header(&hb);
    let end = if with_filter { h.filter_off() + h.filter_blocks as usize * 64 + 4 } else { h.filter_off() };
    let mut b = vec![0u8; end - 64];
    f.read_exact_at(&mut b, 64).unwrap();
    let fl = h.page_count as usize * 8;
    assert_eq!(crc32c(&b[..fl]), le32(&b, fl), "fence crc");
    let fences = b[..fl].chunks_exact(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
    let bloom = with_filter.then(|| {
        let fo = h.filter_off() - 64;
        let flen = h.filter_blocks as usize * 64;
        assert_eq!(crc32c(&b[fo..fo + flen]), le32(&b, fo + flen), "filter crc");
        Bloom::from_bytes(&b[fo..fo + flen], h.k)
    });
    (SegIdx { seg: s, file: f, fences, pages_off: h.pages_off() as u64 }, bloom)
}

struct OpenB {
    segs: Vec<(SegIdx, Bloom)>,
}
fn open_b(dir: &Path) -> OpenB {
    OpenB {
        segs: (0..SEGS)
            .map(|s| {
                let (i, b) = open_seg(dir, s, true);
                (i, b.unwrap())
            })
            .collect(),
    }
}
impl OpenB {
    fn lookup(&self, h: u64, buf: &mut [u8], st: &mut Stats) -> Option<(u32, u32)> {
        for (idx, f) in self.segs.iter().rev() {
            if f.contains(h) {
                st.filter_hits += 1;
                if let Some(r) = idx.search(h, buf, st) {
                    st.found += 1;
                    return Some((idx.seg, r));
                }
                st.fps += 1;
            }
        }
        None
    }
}

struct OpenC {
    global: Bloom,
    segs: Vec<SegIdx>,
}
fn open_c(dir: &Path, mib: usize, k: u32) -> OpenC {
    let f = open_read(&global_path(dir, mib, k));
    let len = f.metadata().unwrap().len() as usize;
    let mut b = vec![0u8; len];
    f.read_exact_at(&mut b, 0).unwrap();
    assert_eq!(&b[0..4], b"SKGF");
    assert_eq!(crc32c(&b[..60]), le32(&b, 60));
    assert_eq!(crc32c(&b[64..len - 4]), le32(&b, len - 4), "global crc");
    let global = Bloom::from_bytes(&b[64..len - 4], le32(&b, 8));
    drop(b);
    OpenC { global, segs: (0..SEGS).map(|s| open_seg(dir, s, false).0).collect() }
}
impl OpenC {
    fn lookup(&self, h: u64, buf: &mut [u8], st: &mut Stats) -> Option<(u32, u32)> {
        if !self.global.contains(h) {
            return None;
        }
        st.filter_hits += 1;
        for idx in self.segs.iter().rev() {
            if let Some(r) = idx.search(h, buf, st) {
                st.found += 1;
                return Some((idx.seg, r));
            }
        }
        st.fps += 1;
        None
    }
}

// ---------- stats helpers ----------
fn summarize(v: &[f64]) -> (f64, f64, f64) {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (s[s.len() / 2], s[0], s[s.len() - 1])
}
fn report(scale: &str, cand: &str, metric: &str, v: &[f64]) {
    let (m, lo, hi) = summarize(v);
    println!("RESULT scale={scale} cand={cand} metric={metric} median={m:.3} min={lo:.3} max={hi:.3} n={}", v.len());
}
fn note(scale: &str, cand: &str, metric: &str, v: f64) {
    println!("RESULT scale={scale} cand={cand} metric={metric} value={v:.6}");
}
fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}
fn rss_kb() -> u64 {
    let o = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().parse().unwrap_or(0)
}

const REPS: usize = 7;

fn canary(tag: &str) {
    let buf = vec![0xa5u8; 1 << 20];
    let mut best = 0f64;
    for _ in 0..20 {
        let t = Instant::now();
        black_box(crc32c(black_box(&buf)));
        best = best.max((1 << 20) as f64 / t.elapsed().as_secs_f64() / 1e9);
    }
    println!("CANARY {tag} crc32c_1MiB_best_GBps={best:.2} (P-core reference 2.16)");
}

fn bench(root: &Path, scale: Scale, cold: bool) {
    canary("start");
    let sn = format!("{}-p{}{}", scale.name, page(), if cold { "-cold" } else { "" });
    let sn = sn.as_str();
    let dir = root.join(sn);
    let n_total = scale.keys_per_seg as u64 * SEGS as u64;
    if !dir.join("seg-0099.kidx").exists() || cold {
        let _ = fs::remove_dir_all(&dir);
        let t = Instant::now();
        generate(&dir, scale, cold);
        println!("INFO gen {sn} {:.0} ms", ms(t));
    }
    NOCACHE.store(cold, Relaxed);
    let mut kb = Vec::new();

    // negatives and positives
    let n_neg: u32 = 1_000_000;
    let negs: Vec<Vec<u8>> = (0..n_neg)
        .map(|j| {
            key(NEG_SEG + j / 1_000_000, j, &mut kb);
            kb.clone()
        })
        .collect();
    let neg_h: Vec<u64> = negs.iter().map(|k| h64(k)).collect();
    let mut rng = 42u64;
    let pos_uniform: Vec<(u32, u32)> = (0..20_000)
        .map(|_| ((splitmix(&mut rng) % SEGS as u64) as u32, (splitmix(&mut rng) % scale.keys_per_seg as u64) as u32))
        .collect();
    let pos_newest: Vec<(u32, u32)> = (0..20_000).map(|_| (SEGS - 1, (splitmix(&mut rng) % scale.keys_per_seg as u64) as u32)).collect();
    let pos_h = |v: &[(u32, u32)]| -> Vec<(u64, u32, u32)> {
        let mut kb = Vec::new();
        v.iter()
            .map(|&(s, i)| {
                key(s, i, &mut kb);
                (h64(&kb), s, i)
            })
            .collect()
    };
    let pu = pos_h(&pos_uniform);
    let pn = pos_h(&pos_newest);

    if !cold {
        // hash cost
        let (mut wy, mut sip) = (vec![], vec![]);
        for _ in 0..REPS {
            let t = Instant::now();
            let mut x = 0u64;
            for k in &negs {
                x ^= h64(black_box(k));
            }
            black_box(x);
            wy.push(t.elapsed().as_nanos() as f64 / n_neg as f64);
            let t = Instant::now();
            let mut x = 0u64;
            for k in &negs {
                x ^= sip13(black_box(k));
            }
            black_box(x);
            sip.push(t.elapsed().as_nanos() as f64 / n_neg as f64);
        }
        report(sn, "hash", "wyhash_style_ns_per_key", &wy);
        report(sn, "hash", "siphash24_ns_per_key", &sip);
    }

    // open timing, interleaved
    let mut open: Vec<(String, Vec<f64>, Vec<f64>)> = vec![];
    let labels: Vec<String> = std::iter::once("A".to_string())
        .chain(std::iter::once("B".to_string()))
        .chain(BUDGETS_MIB.iter().map(|m| format!("C{m}")))
        .collect();
    for l in &labels {
        open.push((l.clone(), vec![], vec![]));
    }
    let reps_a = if cold { 3 } else { REPS };
    for r in 0..REPS {
        for (li, l) in labels.iter().enumerate() {
            if l == "A" && r >= reps_a {
                continue;
            }
            if cold {
                // F_NOCACHE reads do not populate the cache, so every rep re-reads the device.
            }
            let before = LIVE.load(Relaxed);
            let t = Instant::now();
            let mem = match l.as_str() {
                "A" => {
                    let a = open_a(&dir);
                    let el = ms(t);
                    let m = LIVE.load(Relaxed) - before;
                    black_box(a.len());
                    open[li].1.push(el);
                    m
                }
                "B" => {
                    let b = open_b(&dir);
                    let el = ms(t);
                    let m = LIVE.load(Relaxed) - before;
                    black_box(b.segs.len());
                    open[li].1.push(el);
                    m
                }
                c => {
                    let mib: usize = c[1..].parse().unwrap();
                    let o = open_c(&dir, mib, gk(scale, mib));
                    let el = ms(t);
                    let m = LIVE.load(Relaxed) - before;
                    black_box(o.segs.len());
                    open[li].1.push(el);
                    m
                }
            };
            open[li].2.push(mem as f64);
        }
    }
    for (l, t, m) in &open {
        report(sn, l, "open_ms", t);
        let (mm, _, _) = summarize(m);
        note(sn, l, "resident_bytes", mm);
        note(sn, l, "resident_bytes_per_key", mm / n_total as f64);
    }
    if !cold {
        let r0 = rss_kb();
        let a = open_a(&dir);
        let r1 = rss_kb();
        note(sn, "A", "rss_delta_bytes_per_key", (r1.saturating_sub(r0) * 1024) as f64 / n_total as f64);
        note(sn, "A", "hashmap_capacity", a.capacity() as f64);
        // correctness: positives found at right place, negatives miss
        let mut bad = 0;
        for &(h, s, i) in pu.iter().chain(&pn) {
            if a.get(&h) != Some(&((s as u64) << 32 | i as u64)) {
                bad += 1;
            }
        }
        let neg_hits = neg_h.iter().filter(|h| a.contains_key(h)).count();
        note(sn, "A", "positive_mismatches", bad as f64);
        note(sn, "A", "negative_hash_hits", neg_hits as f64);
        let mut tn = vec![];
        let mut tp = vec![];
        for _ in 0..REPS {
            let t = Instant::now();
            let mut c = 0usize;
            for h in &neg_h {
                c += a.contains_key(black_box(h)) as usize;
            }
            black_box(c);
            tn.push(t.elapsed().as_nanos() as f64 / n_neg as f64);
            let t = Instant::now();
            let mut c = 0u64;
            for (h, _, _) in &pu {
                c ^= a.get(black_box(h)).copied().unwrap_or(0);
            }
            black_box(c);
            tp.push(t.elapsed().as_nanos() as f64 / pu.len() as f64);
        }
        report(sn, "A", "neg_lookup_ns", &tn);
        report(sn, "A", "pos_lookup_ns_excl_frame_read", &tp);
        drop(a);
    }

    // B and C lookups
    let b = open_b(&dir);
    let cs: Vec<OpenC> = BUDGETS_MIB.iter().map(|&m| open_c(&dir, m, gk(scale, m))).collect();
    let mut buf = vec![0u8; page()];
    // correctness
    {
        let mut bad = 0;
        let mut st = Stats::default();
        for &(h, s, i) in pu.iter().chain(&pn) {
            if b.lookup(h, &mut buf, &mut st) != Some((s, i)) {
                bad += 1;
            }
            for c in &cs {
                if c.lookup(h, &mut buf, &mut st) != Some((s, i)) {
                    bad += 1;
                }
            }
        }
        note(sn, "B+C", "positive_mismatches", bad as f64);
        let mut st = Stats::default();
        let mut found = 0;
        for &h in neg_h.iter().take(100_000) {
            found += b.lookup(h, &mut buf, &mut st).is_some() as u32;
            for c in &cs {
                found += c.lookup(h, &mut buf, &mut st).is_some() as u32;
            }
        }
        note(sn, "B+C", "negative_found", found as f64);
    }
    let n_neg_timed = if cold { 20_000usize } else { n_neg as usize };
    let mut tb = vec![];
    let mut tbf = vec![];
    let mut tc: Vec<Vec<f64>> = vec![vec![]; cs.len()];
    let mut stb = Stats::default();
    let mut stc = vec![Stats::default(); cs.len()];
    let mut td: Vec<Vec<f64>> = vec![vec![]; cs.len()];
    let mut std_ = vec![Stats::default(); cs.len()];
    for r in 0..REPS {
        let mut st = Stats::default();
        let t = Instant::now();
        for &h in &neg_h[..n_neg_timed] {
            black_box(b.lookup(black_box(h), &mut buf, &mut st));
        }
        tb.push(t.elapsed().as_nanos() as f64 / n_neg_timed as f64);
        if r == 0 {
            stb = st;
        }
        // filter-only probe cost for B
        let t = Instant::now();
        let mut c = 0u32;
        for &h in &neg_h[..n_neg_timed] {
            for (_, f) in b.segs.iter().rev() {
                c += f.contains(black_box(h)) as u32;
            }
        }
        black_box(c);
        tbf.push(t.elapsed().as_nanos() as f64 / n_neg_timed as f64);
        for (ci, c) in cs.iter().enumerate() {
            let mut st = Stats::default();
            let t = Instant::now();
            for &h in &neg_h[..n_neg_timed] {
                black_box(c.lookup(black_box(h), &mut buf, &mut st));
            }
            tc[ci].push(t.elapsed().as_nanos() as f64 / n_neg_timed as f64);
            if r == 0 {
                stc[ci] = st;
            }
            // D: the global filter in front of B's per-segment filters
            let mut st = Stats::default();
            let t = Instant::now();
            for &h in &neg_h[..n_neg_timed] {
                let h = black_box(h);
                if c.global.contains(h) {
                    black_box(b.lookup(h, &mut buf, &mut st));
                }
            }
            td[ci].push(t.elapsed().as_nanos() as f64 / n_neg_timed as f64);
            if r == 0 {
                std_[ci] = st;
            }
        }
    }
    for (ci, &m) in BUDGETS_MIB.iter().enumerate() {
        let l = format!("D{m}");
        report(sn, &l, "neg_lookup_ns_end_to_end", &td[ci]);
        note(sn, &l, "neg_preads_per_lookup", std_[ci].preads as f64 / n_neg_timed as f64);
    }
    report(sn, "B", "neg_lookup_ns_end_to_end", &tb);
    report(sn, "B", "neg_filter_probe_only_ns", &tbf);
    note(sn, "B", "neg_filter_hits_per_lookup", stb.filter_hits as f64 / n_neg_timed as f64);
    note(sn, "B", "neg_preads_per_lookup", stb.preads as f64 / n_neg_timed as f64);
    note(sn, "B", "neg_any_found", stb.found as f64);
    for (ci, &m) in BUDGETS_MIB.iter().enumerate() {
        let l = format!("C{m}");
        report(sn, &l, "neg_lookup_ns_end_to_end", &tc[ci]);
        note(sn, &l, "neg_fpr_measured", stc[ci].fps as f64 / n_neg_timed as f64);
        note(sn, &l, "neg_preads_per_lookup", stc[ci].preads as f64 / n_neg_timed as f64);
        note(sn, &l, "k", gk(scale, m) as f64);
    }

    // positives
    let n_pos = if cold { 2_000 } else { pu.len() };
    for (pname, set) in [("uniform", &pu), ("newest", &pn)] {
        let mut tbp = vec![];
        let mut tcp = vec![];
        let mut sb = Stats::default();
        let mut sc = Stats::default();
        for r in 0..REPS {
            let mut st = Stats::default();
            let t = Instant::now();
            for &(h, _, _) in &set[..n_pos] {
                black_box(b.lookup(h, &mut buf, &mut st));
            }
            tbp.push(t.elapsed().as_nanos() as f64 / n_pos as f64);
            if r == 0 {
                sb = st;
            }
            // positive cost in C does not depend on the filter budget once the filter says maybe; use C16.
            let mut st = Stats::default();
            let t = Instant::now();
            for &(h, _, _) in &set[..n_pos] {
                black_box(cs[1].lookup(h, &mut buf, &mut st));
            }
            tcp.push(t.elapsed().as_nanos() as f64 / n_pos as f64);
            if r == 0 {
                sc = st;
            }
        }
        report(sn, "B", &format!("pos_{pname}_ns_excl_frame_read"), &tbp);
        note(sn, "B", &format!("pos_{pname}_preads"), sb.preads as f64 / n_pos as f64);
        report(sn, "C", &format!("pos_{pname}_ns_excl_frame_read"), &tcp);
        note(sn, "C", &format!("pos_{pname}_preads"), sc.preads as f64 / n_pos as f64);
    }

    // C false-positive cost: search every segment index for an absent hash
    {
        let n = if cold { 200 } else { 5_000 };
        let mut v = vec![];
        let mut preads = 0;
        for _ in 0..REPS {
            let mut st = Stats::default();
            let t = Instant::now();
            for &h in &neg_h[..n] {
                for idx in cs[0].segs.iter().rev() {
                    black_box(idx.search(h, &mut buf, &mut st));
                }
            }
            v.push(t.elapsed().as_nanos() as f64 / n as f64 / 1e3);
            preads = st.preads;
        }
        report(sn, "C", "false_positive_cost_us_all_100_segments", &v);
        note(sn, "C", "false_positive_preads", preads as f64 / n as f64);
        // single page probe cost (one pread + crc + bsearch)
        let mut v = vec![];
        for _ in 0..REPS {
            let mut st = Stats::default();
            let t = Instant::now();
            for &h in &neg_h[..n] {
                black_box(cs[0].segs[0].search(h, &mut buf, &mut st));
            }
            v.push(t.elapsed().as_nanos() as f64 / st.preads as f64);
        }
        report(sn, "BC", "one_index_page_probe_ns", &v);
        let t = Instant::now();
        let mut x = 0;
        for _ in 0..100_000 {
            x ^= crc32c(black_box(&buf[..page() - 4]));
        }
        black_box(x);
        note(sn, "BC", "crc_of_one_page_ns", t.elapsed().as_nanos() as f64 / 100_000.0);
    }
    drop(b);
    drop(cs);
    NOCACHE.store(false, Relaxed);
    if cold {
        return;
    }

    // FPR tables (in-memory filters built from the same hashes)
    let mut seg_hashes: Vec<Vec<u64>> = vec![];
    for s in 0..SEGS {
        seg_hashes.push(segment_entries(s, scale.keys_per_seg, &mut kb).into_iter().map(|e| e.0).collect());
    }
    for &m in &BUDGETS_MIB {
        for k in [gk(scale, m), 4, 8] {
            let mut g = Bloom::with_bytes(m << 20, k);
            for hs in &seg_hashes {
                for &h in hs {
                    g.insert(h);
                }
            }
            let fp = neg_h.iter().filter(|&&h| g.contains(h)).count();
            note(sn, &format!("C{m}"), &format!("fpr_k{k}"), fp as f64 / n_neg as f64);
        }
        note(sn, &format!("C{m}"), "bits_per_key", (m * 8 << 20) as f64 / n_total as f64);
    }
    for bpk in [10.0, 16.0, 20.0] {
        let k = k_opt(bpk);
        let filters: Vec<Bloom> = seg_hashes
            .iter()
            .map(|hs| {
                let mut f = Bloom::for_keys(hs.len(), bpk, k);
                hs.iter().for_each(|&h| f.insert(h));
                f
            })
            .collect();
        let (mut hits, mut any) = (0u64, 0u64);
        for &h in &neg_h {
            let c = filters.iter().filter(|f| f.contains(h)).count() as u64;
            hits += c;
            any += (c > 0) as u64;
        }
        note(sn, "B", &format!("per_segment_fpr_bpk{bpk}_k{k}"), hits as f64 / (n_neg as f64 * SEGS as f64));
        note(sn, "B", &format!("p_any_of_100_hits_bpk{bpk}"), any as f64 / n_neg as f64);
    }

    // roll-time cost for one new segment
    {
        let n = scale.keys_per_seg;
        let keys: Vec<Vec<u8>> = (0..n)
            .map(|i| {
                key(SEGS, i, &mut kb);
                kb.clone()
            })
            .collect();
        let mut globals: Vec<Bloom> = BUDGETS_MIB.iter().map(|&m| Bloom::with_bytes(m << 20, gk(scale, m))).collect();
        for hs in &seg_hashes {
            for &h in hs {
                for g in &mut globals {
                    g.insert(h);
                }
            }
        }
        let (mut th, mut ts, mut tf, mut tser, mut tw, mut tws) = (vec![], vec![], vec![], vec![], vec![], vec![]);
        let mut tg: Vec<(Vec<f64>, Vec<f64>, Vec<f64>)> = vec![(vec![], vec![], vec![]); BUDGETS_MIB.len()];
        let mut side_len = 0;
        let roll = root.join("roll");
        fs::create_dir_all(&roll).unwrap();
        for _ in 0..REPS {
            let t = Instant::now();
            let mut e: Vec<(u64, u32)> = keys.iter().enumerate().map(|(i, k)| (h64(k), i as u32)).collect();
            th.push(ms(t));
            let t = Instant::now();
            e.sort_unstable();
            ts.push(ms(t));
            let t = Instant::now();
            let mut f = Bloom::for_keys(e.len(), bbpk(), bk());
            e.iter().for_each(|&(h, _)| f.insert(h));
            tf.push(ms(t));
            let t = Instant::now();
            let side = build_sidecar(SEGS, &e, &f);
            tser.push(ms(t));
            side_len = side.len();
            let t = Instant::now();
            write_file(&roll.join("new.kidx"), &side, false, false);
            tw.push(ms(t));
            let t = Instant::now();
            write_file(&roll.join("new.kidx"), &side, true, false);
            tws.push(ms(t));
            for (gi, g) in globals.iter_mut().enumerate() {
                let t = Instant::now();
                e.iter().for_each(|&(h, _)| g.insert(h));
                tg[gi].0.push(ms(t));
                let t = Instant::now();
                let bytes = global_bytes(g, 0);
                tg[gi].1.push(ms(t));
                let t = Instant::now();
                let tmp = roll.join("global.tmp");
                write_file(&tmp, &bytes, true, false);
                fs::rename(&tmp, roll.join("global.kflt")).unwrap();
                File::open(&roll).unwrap().sync_all().unwrap();
                tg[gi].2.push(ms(t));
            }
        }
        report(sn, "roll", "hash_keys_ms", &th);
        report(sn, "roll", "sort_ms", &ts);
        report(sn, "roll", "bloom_build_B_ms", &tf);
        report(sn, "roll", "serialize_crc_ms", &tser);
        report(sn, "roll", "write_nosync_ms", &tw);
        report(sn, "roll", "write_sync_all_ms", &tws);
        note(sn, "roll", "sidecar_bytes_per_key_with_filter", side_len as f64 / n as f64);
        let filt = Bloom::for_keys(n as usize, bbpk(), bk()).bytes() + 4;
        note(sn, "roll", "sidecar_bytes_per_key_without_filter", (side_len - filt) as f64 / n as f64);
        for (gi, &m) in BUDGETS_MIB.iter().enumerate() {
            report(sn, &format!("C{m}"), "roll_or_into_global_ms", &tg[gi].0);
            report(sn, &format!("C{m}"), "roll_serialize_crc_global_ms", &tg[gi].1);
            report(sn, &format!("C{m}"), "roll_write_sync_rename_dirsync_ms", &tg[gi].2);
        }
    }

    // rebuild costs
    {
        let seg = root.join("roll/segment-64MiB.bin");
        let mut data = vec![0u8; 64 << 20];
        let mut x = 7u64;
        for c in data.chunks_exact_mut(8) {
            c.copy_from_slice(&splitmix(&mut x).to_le_bytes());
        }
        write_file(&seg, &data, true, false);
        let (mut tr, mut tcr, mut tcold) = (vec![], vec![], vec![]);
        for r in 0..REPS {
            let t = Instant::now();
            let f = File::open(&seg).unwrap();
            f.read_exact_at(&mut data, 0).unwrap();
            tr.push(ms(t));
            let t = Instant::now();
            black_box(crc32c(&data));
            tcr.push(ms(t));
            let p = root.join(format!("roll/cold-{r}.bin"));
            write_file(&p, &data, true, true);
            let f = File::open(&p).unwrap();
            set_nocache(&f);
            let t = Instant::now();
            f.read_exact_at(&mut data, 0).unwrap();
            tcold.push(ms(t));
            fs::remove_file(&p).unwrap();
        }
        report(sn, "rebuild", "read_64MiB_warm_ms", &tr);
        report(sn, "rebuild", "read_64MiB_nocache_ms", &tcold);
        report(sn, "rebuild", "crc32c_64MiB_ms", &tcr);
        // C: rebuild the global filter from every sidecar index
        let mut tgr = vec![];
        for _ in 0..REPS {
            let t = Instant::now();
            let a = {
                let mut g = Bloom::with_bytes(16 << 20, gk(scale, 16));
                let mut buf = Vec::new();
                for s in 0..SEGS {
                    let f = File::open(seg_path(&dir, s)).unwrap();
                    let mut hb = [0u8; 64];
                    f.read_exact_at(&mut hb, 0).unwrap();
                    let h = parse_header(&hb);
                    let pg = page();
                    buf.resize(h.page_count as usize * pg, 0);
                    f.read_exact_at(&mut buf, h.pages_off() as u64).unwrap();
                    for page in buf.chunks_exact(pg) {
                        assert_eq!(crc32c(&page[..pg - 4]), le32(page, pg - 4));
                        for i in 0..le32(page, pg - 16) as usize {
                            g.insert(le64(page, i * 12));
                        }
                    }
                }
                g
            };
            black_box(a.blocks.len());
            tgr.push(ms(t));
        }
        report(sn, "C16", "rebuild_global_from_sidecars_ms", &tgr);
    }

    // baseline A0: today's BTreeMap<Vec<u8>, Position>
    {
        let before = LIVE.load(Relaxed);
        let t = Instant::now();
        let mut m: BTreeMap<Vec<u8>, (u64, u64)> = BTreeMap::new();
        for s in 0..SEGS {
            for i in 0..scale.keys_per_seg {
                key(s, i, &mut kb);
                m.insert(kb.clone(), (s as u64, i as u64));
            }
        }
        let build = ms(t);
        let mem = LIVE.load(Relaxed) - before;
        note(sn, "A0", "build_from_keys_ms", build);
        note(sn, "A0", "resident_bytes_per_key", mem as f64 / n_total as f64);
        let mut tn = vec![];
        for _ in 0..5 {
            let t = Instant::now();
            let mut c = 0;
            for k in &negs {
                c += m.contains_key(black_box(k)) as usize;
            }
            assert_eq!(c, 0);
            tn.push(t.elapsed().as_nanos() as f64 / n_neg as f64);
        }
        report(sn, "A0", "neg_lookup_ns", &tn);
    }
}

fn coldtest(root: &Path) {
    // Does F_NOCACHE give device latency? Compare random 4 KiB preads.
    fs::create_dir_all(root).unwrap();
    let data = vec![0x5au8; 256 << 20];
    let p1 = root.join("ct-normal.bin");
    let p2 = root.join("ct-nocache.bin");
    write_file(&p1, &data, true, false);
    write_file(&p2, &data, true, true);
    let mut buf = [0u8; PAGE];
    for (name, p, nc) in [("written-normal read-normal", &p1, false), ("written-normal read-nocache", &p1, true), ("written-nocache read-nocache", &p2, true)] {
        let f = File::open(p).unwrap();
        if nc {
            set_nocache(&f);
        }
        let mut x = 99u64;
        let n = 2000;
        let t = Instant::now();
        for _ in 0..n {
            let page = splitmix(&mut x) % ((256 << 20) / PAGE as u64);
            f.read_exact_at(&mut buf, page * PAGE as u64).unwrap();
        }
        println!("COLDTEST {name}: {:.2} us per random 4 KiB pread", t.elapsed().as_secs_f64() * 1e6 / n as f64);
    }
    fs::remove_file(p1).unwrap();
    fs::remove_file(p2).unwrap();
}

fn main() {
    assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    let args: Vec<String> = std::env::args().collect();
    if let Some(p) = args.get(3) {
        PAGE_SZ.store(p.parse().unwrap(), Relaxed);
    }
    let root = PathBuf::from(&args[2]);
    let s1 = Scale { name: "S1", keys_per_seg: 200_000 };
    let s2 = Scale { name: "S2", keys_per_seg: 20_000 };
    match args[1].as_str() {
        "coldtest" => coldtest(&root),
        "crcbench" => {
            for size in [508usize, 1020, 4092, 1 << 20, 64 << 20] {
                let buf: Vec<u8> = (0..size).map(|i| (i * 7) as u8).collect();
                let iters = (256usize << 20) / size;
                let mut v = vec![];
                for _ in 0..REPS {
                    let t = Instant::now();
                    let mut x = 0u32;
                    for _ in 0..iters {
                        x ^= crc32c(black_box(&buf));
                    }
                    black_box(x);
                    v.push((size * iters) as f64 / t.elapsed().as_secs_f64() / 1e9);
                }
                let (m, lo, hi) = summarize(&v);
                println!("CRC size={size} GB/s median={m:.2} min={lo:.2} max={hi:.2} ns_per_call={:.0}", size as f64 / m);
            }
        }
        "S1" => bench(&root, s1, false),
        "S2" => bench(&root, s2, false),
        "S1-cold" => bench(&root, s1, true),
        "S2-cold" => bench(&root, s2, true),
        "canary" => {}
        "neg" => negonly(&root, s1),
        _ => panic!("usage: m4-proto <coldtest|S1|S2|S1-cold|S2-cold> <data-dir>"),
    }
    canary("end");
}

fn negonly(root: &Path, scale: Scale) {
    canary("start");
    let dir = root.join(format!("S1-bpk{}", bbpk()));
    if !dir.join("seg-0099.kidx").exists() {
        generate(&dir, scale, false);
    }
    let mut kb = Vec::new();
    let neg_h: Vec<u64> = (0..1_000_000u32).map(|j| { key(NEG_SEG, j, &mut kb); h64(&kb) }).collect();
    let mut topen = vec![];
    let mut mem = 0;
    for _ in 0..REPS {
        let before = LIVE.load(Relaxed);
        let t = Instant::now();
        let b = open_b(&dir);
        topen.push(ms(t));
        mem = LIVE.load(Relaxed) - before;
        black_box(b.segs.len());
    }
    let tag = format!("S1-bpk{}", bbpk());
    report(&tag, "B", "open_ms", &topen);
    note(&tag, "B", "resident_bytes", mem as f64);
    let b = open_b(&dir);
    let c16 = open_c(&dir, 16, gk(scale, 16));
    let mut buf = vec![0u8; page()];
    let (mut tb, mut td) = (vec![], vec![]);
    let (mut sb, mut sd) = (Stats::default(), Stats::default());
    for r in 0..REPS {
        let mut st = Stats::default();
        let t = Instant::now();
        for &h in &neg_h { black_box(b.lookup(black_box(h), &mut buf, &mut st)); }
        tb.push(t.elapsed().as_nanos() as f64 / neg_h.len() as f64);
        if r == 0 { sb = st; }
        let mut st = Stats::default();
        let t = Instant::now();
        for &h in &neg_h { let h = black_box(h); if c16.global.contains(h) { black_box(b.lookup(h, &mut buf, &mut st)); } }
        td.push(t.elapsed().as_nanos() as f64 / neg_h.len() as f64);
        if r == 0 { sd = st; }
    }
    report(&tag, "B", "neg_lookup_ns_end_to_end", &tb);
    note(&tag, "B", "neg_preads_per_lookup", sb.preads as f64 / neg_h.len() as f64);
    report(&tag, "D16", "neg_lookup_ns_end_to_end", &td);
    note(&tag, "D16", "neg_preads_per_lookup", sd.preads as f64 / neg_h.len() as f64);
}
