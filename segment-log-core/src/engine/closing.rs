//! A roll's closing run (format.md, rules for readers): the frames that seal a segment, and how a
//! forward scan recognizes them.
use super::*;
use survey::ReadOnly;

/// One stream's last record in a segment: a footer entry before encoding.
#[derive(Clone, Copy, Debug)]
pub(super) struct Tail {
    pub revision: u64,
    pub offset: u64,
    /// The file position of the group frame that holds the record.
    pub position: u64,
}

/// A roll's closing run for a segment whose frames end at `filter_at`, in write order: key filter,
/// key index, footer and locator.
pub(super) fn roll_run(
    log_id: &codec::LogId,
    epoch: u64,
    next: u64,
    filter_at: u64,
    tails: &BTreeMap<Vec<u8>, Tail>,
    keys: &mut [codec::KeyEntry],
) -> [Vec<u8>; 4] {
    let codec::KeyFrames { filter, index } = codec::encode_key_frames(log_id, epoch, next, keys)
        .expect("the writer rolls before the key index passes the frame limit");
    let footer = codec::encode_footer(
        log_id,
        epoch,
        next,
        &codec::Footer {
            key_filter_position: std::num::NonZeroU64::new(filter_at),
            streams: tails
                .iter()
                .map(|(stream, tail)| codec::StreamTail {
                    stream,
                    last_revision: tail.revision,
                    last_offset: tail.offset,
                    last_position: tail.position,
                })
                .collect(),
        },
    )
    .expect("the writer rolls before the footer passes the frame limit");
    let footer_at = filter_at + (filter.len() + index.len()) as u64;
    let locator = codec::encode_locator(log_id, epoch, next, footer_at);
    [filter, index, footer, locator]
}

/// Where a forward scan is in a roll's closing run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RunState {
    Groups,
    /// After a key filter at `start` for `count` keyed records.
    Filter {
        start: u64,
        count: u32,
    },
    /// After the key index that matches the key filter at `start`.
    Index {
        start: u64,
    },
    /// After the footer at `footer`, which names the key filter at `start`.
    Footer {
        start: u64,
        footer: u64,
    },
}

pub(super) enum Step {
    /// A group frame continues the segment's groups.
    Group,
    /// A closing frame continues a run.
    Run(RunState),
    /// The log ends at `at`: the frame's position, or the start of a run that it cannot continue.
    Stop { at: u64 },
    /// A complete roll's run from `start` ends the segment.
    Closed { start: u64 },
}

impl RunState {
    /// The step for a frame at `at` that ends at `end`, in a segment of `length` bytes. The caller
    /// has checked the frame's offset and epoch continuity.
    pub(super) fn step(self, at: u64, end: u64, length: u64, body: &codec::Body<'_>) -> Step {
        use codec::Body;
        match (self, body) {
            (Self::Groups, Body::Group(_)) => Step::Group,
            (Self::Groups, Body::KeyFilter(filter)) => Step::Run(Self::Filter {
                start: at,
                count: filter.count(),
            }),
            (Self::Filter { start, count }, Body::KeyIndex { count: n, .. }) if *n == count => {
                Step::Run(Self::Index { start })
            }
            (Self::Index { start }, Body::Footer(footer))
                if footer.key_filter_position.map(u64::from) == Some(start) =>
            {
                Step::Run(Self::Footer { start, footer: at })
            }
            (Self::Footer { start, footer }, Body::Locator { footer_position })
                if *footer_position == footer && end == length =>
            {
                Step::Closed { start }
            }
            _ => Step::Stop { at: self.stop(at) },
        }
    }

    /// Where the log ends when the frame at `at` is not valid, or the file ends at `at`.
    pub(super) fn stop(self, at: u64) -> u64 {
        match self {
            Self::Groups => at,
            Self::Filter { start, .. } | Self::Index { start } | Self::Footer { start, .. } => {
                start
            }
        }
    }
}

/// Open step 7, for a newest segment whose scan stopped at `at` before its end: whether the last
/// 40 bytes are a valid locator that names a valid footer whose run starts after `at`. Then the
/// segment is sealed and the frame at `at` is corruption, not a torn tail.
pub(super) fn sealed_behind(
    storage: &ReadOnly<'_>,
    path: &Path,
    log_id: &codec::LogId,
    length: u64,
    at: u64,
) -> io::Result<bool> {
    let Some(locator_at) = length.checked_sub(codec::LOCATOR_LEN as u64) else {
        return Ok(false);
    };
    let bytes = storage.read(path, Purpose::Summary, locator_at, codec::LOCATOR_LEN)?;
    let Ok(codec::Body::Locator { footer_position }) =
        codec::decode(log_id, &bytes).map(|f| f.body)
    else {
        return Ok(false);
    };
    let Some(footer) = storage.frame(path, Purpose::Summary, footer_position, locator_at)? else {
        return Ok(false);
    };
    let Ok(codec::Body::Footer(footer)) = codec::decode(log_id, &footer).map(|f| f.body) else {
        return Ok(false);
    };
    Ok(at
        < footer
            .key_filter_position
            .map_or(footer_position, u64::from))
}
