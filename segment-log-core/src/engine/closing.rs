use super::*;
use survey::ReadOnly;

#[derive(Clone, Copy, Debug)]
pub(super) struct Tail {
    pub revision: u64,
    pub offset: u64,
    pub position: u64,
}

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
    let footer = footer(
        log_id,
        epoch,
        next,
        std::num::NonZeroU64::new(filter_at),
        tails,
    );
    let footer_at = filter_at + (filter.len() + index.len()) as u64;
    let locator = codec::encode_locator(log_id, epoch, next, footer_at);
    [filter, index, footer, locator]
}

/// No key frames: a sealed log never checks a key again (ADR 0048).
pub(super) fn seal_run(
    log_id: &codec::LogId,
    epoch: u64,
    next: u64,
    footer_at: u64,
    tails: &BTreeMap<Vec<u8>, Tail>,
) -> [Vec<u8>; 3] {
    [
        footer(log_id, epoch, next, None, tails),
        codec::encode_seal(log_id, epoch, next),
        codec::encode_locator(log_id, epoch, next, footer_at),
    ]
}

fn footer(
    log_id: &codec::LogId,
    epoch: u64,
    next: u64,
    key_filter_position: Option<std::num::NonZeroU64>,
    tails: &BTreeMap<Vec<u8>, Tail>,
) -> Vec<u8> {
    let streams = tails
        .iter()
        .map(|(stream, tail)| codec::StreamTail {
            stream,
            last_revision: tail.revision,
            last_offset: tail.offset,
            last_position: tail.position,
        })
        .collect();
    let footer = codec::Footer {
        key_filter_position,
        streams,
    };
    codec::encode_footer(log_id, epoch, next, &footer)
        .expect("the writer rolls before the footer passes the frame limit")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RunState {
    Groups,
    Filter { start: u64, count: u32 },
    Index { start: u64 },
    Footer { start: u64, footer: u64 },
    SealFooter { footer: u64 },
    Seal { footer: u64 },
}

pub(super) enum Step {
    Group,
    Run(RunState),
    Stop { at: u64 },
    Rolled { start: u64, footer: u64 },
    Sealed { start: u64 },
}

impl RunState {
    pub(super) fn step(self, at: u64, end: u64, length: u64, body: &codec::Body<'_>) -> Step {
        use codec::Body;
        match (self, body) {
            (Self::Groups, Body::Group(_)) => Step::Group,
            (Self::Groups, Body::KeyFilter(filter)) => Step::Run(Self::Filter {
                start: at,
                count: filter.count(),
            }),
            (Self::Groups, Body::Footer(footer)) if footer.key_filter_position.is_none() => {
                Step::Run(Self::SealFooter { footer: at })
            }
            (Self::Filter { start, count }, Body::KeyIndex { count: n, .. }) if *n == count => {
                Step::Run(Self::Index { start })
            }
            (Self::Index { start }, Body::Footer(footer))
                if footer.key_filter_position.map(u64::from) == Some(start) =>
            {
                Step::Run(Self::Footer { start, footer: at })
            }
            (Self::SealFooter { footer }, Body::Seal) => Step::Run(Self::Seal { footer }),
            (Self::Footer { start, footer }, Body::Locator { footer_position })
                if *footer_position == footer && end == length =>
            {
                Step::Rolled { start, footer }
            }
            (Self::Seal { footer }, Body::Locator { footer_position })
                if *footer_position == footer && end == length =>
            {
                Step::Sealed { start: footer }
            }
            _ => Step::Stop { at: self.stop(at) },
        }
    }

    pub(super) fn stop(self, at: u64) -> u64 {
        match self {
            Self::Groups => at,
            Self::Filter { start, .. } | Self::Index { start } | Self::Footer { start, .. } => {
                start
            }
            Self::SealFooter { footer } | Self::Seal { footer } => footer,
        }
    }
}

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
