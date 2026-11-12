use super::*;
use closing::Tail;
use keys::Classification;

pub(super) struct Writer {
    pub epoch: u64,
    pub keys: Keys,
    pub active: Active,
    pub limits: Limits,
    #[cfg(test)]
    pub memory: Arc<Mutex<KeyMemory>>,
}

pub(super) struct Active {
    pub base: u64,
    pub length: u64,
    tails: BTreeMap<Vec<u8>, Tail>,
    footer_body: usize,
}

impl Active {
    pub(super) fn new(base: u64) -> Self {
        Self::recovered(base, codec::HEADER_LEN as u64, BTreeMap::new())
    }

    pub(super) fn recovered(base: u64, length: u64, tails: BTreeMap<Vec<u8>, Tail>) -> Self {
        let footer_body = tails
            .keys()
            .map(|stream| footer_entry(stream))
            .sum::<usize>();
        Self {
            base,
            length,
            tails,
            footer_body: codec::FOOTER_HEADER_LEN + footer_body,
        }
    }
}

fn footer_entry(stream: &[u8]) -> usize {
    codec::FOOTER_ENTRY_LEN + stream.len()
}

impl Writer {
    fn fits(&self, draft: &Draft, submit: &Submit) -> bool {
        let mut streams: Vec<&[u8]> = submit
            .records
            .iter()
            .map(|record| record.stream.as_slice())
            .filter(|stream| !self.active.tails.contains_key(*stream))
            .filter(|stream| !draft.tails.contains_key(*stream))
            .collect();
        streams.sort_unstable();
        streams.dedup();
        let footer = self.active.footer_body
            + draft.footer_growth
            + streams.into_iter().map(footer_entry).sum::<usize>();
        let keyed = submit.records.iter().filter(|r| r.key.is_some()).count();
        footer <= self.limits.footer_body && self.keys.keyed() + keyed <= self.limits.keyed_records
    }

    fn rolls_before(&self, next: u64, fits: bool, options: &Options) -> bool {
        // A segment with no record cannot roll: its successor would take its name.
        (self.active.length >= options.roll_bytes || !fits) && next != self.active.base
    }

    /// The writer never appends to a segment after its first closing frame.
    fn roll(&mut self, view: &View, next: u64) -> io::Result<()> {
        let storage = view.storage.as_ref();
        let path = segment_path(&view.dir, self.active.base);
        let mut keys = self.keys.index_entries();
        let filter_at = self.active.length;
        let run = closing::roll_run(
            &view.log_id,
            self.epoch,
            next,
            filter_at,
            &self.active.tails,
            &mut keys,
        );
        let summary = keys::SealedSummary {
            base: self.active.base,
            next,
            filter_at,
            footer_at: filter_at + (run[0].len() + run[1].len()) as u64,
        };
        let sealed = keys::SealedKeys::written(summary, &view.log_id, &run[0]);
        for frame in run {
            call(storage, &path, Purpose::Roll, Op::Append(frame))?;
        }
        call(storage, &path, Purpose::Roll, Op::SyncData)?;
        let header = codec::Header {
            log_id: *view.log_id.bytes(),
            base_offset: next,
            epoch: self.epoch,
        };
        create_segment(storage, &view.dir, Purpose::Roll, &header)?;
        self.keys.roll(sealed);
        view.published.lock().unwrap().roll();
        self.active = Active::new(next);
        Ok(())
    }

    fn seal(&mut self, view: &View) -> io::Result<()> {
        let storage = view.storage.as_ref();
        let path = segment_path(&view.dir, self.active.base);
        let run = closing::seal_run(
            &view.log_id,
            self.epoch,
            view.watermark(),
            self.active.length,
            &self.active.tails,
        );
        for frame in run {
            call(storage, &path, Purpose::Seal, Op::Append(frame))?;
        }
        call(storage, &path, Purpose::Seal, Op::SyncData)?;
        Ok(())
    }
}

pub(super) struct RevisionSlot {
    pub index: usize,
    pub revision: Revision,
}
// Classification reuses the result-position allowance before any result is allocated.
const _: () = assert!(std::mem::size_of::<RevisionSlot>() <= std::mem::size_of::<Position>());
/// `pending` holds the revisions that the current group has assigned. It overrides the
/// durable heads, which the group has not changed yet.
fn revisions(
    submit: &Submit,
    pending: &BTreeMap<Vec<u8>, Tail>,
    published: &Mutex<Published>,
) -> Result<Box<[RevisionSlot]>, CommitError> {
    let mut slots: Box<[_]> = (0..submit.records.len())
        .map(|index| RevisionSlot { index, revision: 0 })
        .collect();
    slots.sort_unstable_by(|a, b| {
        submit.records[a.index]
            .stream
            .cmp(&submit.records[b.index].stream)
            .then(a.index.cmp(&b.index))
    });
    let mut cursor = 0;
    let mut failure: Option<(usize, CommitError)> = None;
    // Only the writer thread changes the durable heads, and it does so between groups.
    let durable = published.lock().unwrap();
    while cursor < slots.len() {
        let stream = &submit.records[slots[cursor].index].stream;
        let mut actual = pending
            .get(stream)
            .map(|tail| tail.revision)
            .or_else(|| durable.heads.get(stream).map(|head| head.revision));
        while cursor < slots.len() && submit.records[slots[cursor].index].stream == *stream {
            let index = slots[cursor].index;
            let matches = match submit.records[index].expected {
                ExpectedRevision::Empty => actual.is_none(),
                ExpectedRevision::At(n) => actual == Some(n),
            };
            let result = if !matches {
                Err(CommitError::WrongRevision { index, actual })
            } else {
                match actual {
                    None => Ok(0),
                    Some(n) => n.checked_add(1).ok_or(CommitError::Exhausted),
                }
            };
            let revision = match result {
                Ok(revision) => revision,
                Err(error) => {
                    if failure.as_ref().is_none_or(|(earlier, _)| index < *earlier) {
                        failure = Some((index, error));
                    }
                    while cursor < slots.len()
                        && submit.records[slots[cursor].index].stream == *stream
                    {
                        cursor += 1;
                    }
                    break;
                }
            };
            slots[cursor].revision = revision;
            actual = Some(revision);
            cursor += 1;
        }
    }
    if let Some((_, error)) = failure {
        return Err(error);
    }
    slots.sort_unstable_by_key(|slot| slot.index);
    Ok(slots)
}

struct Draft {
    base: u64,
    requests: Vec<Request>,
    /// Request and record index of each staged record, by offset past `base`.
    origins: Vec<(usize, usize)>,
    assigned: Vec<Revision>,
    tails: BTreeMap<Vec<u8>, Tail>,
    footer_growth: usize,
    waiters: Vec<RetryWaiter>,
    size: usize,
}

impl Draft {
    fn new(base: u64) -> Self {
        Self {
            base,
            requests: Vec::new(),
            origins: Vec::new(),
            assigned: Vec::new(),
            // Copying every durable head here would make each group cost grow with the stream count.
            tails: BTreeMap::new(),
            footer_growth: 0,
            waiters: Vec::new(),
            size: codec::BODY_HEADER_LEN,
        }
    }

    fn staged(&self, position: u64) -> keys::Staged<'_> {
        keys::Staged {
            position,
            base: self.base,
            requests: &self.requests,
            origins: &self.origins,
            assigned: &self.assigned,
        }
    }
}

impl Writer {
    fn stage(&mut self, draft: &mut Draft, request: Request, revisions: &[RevisionSlot]) {
        let submit = &request.submit;
        let first = draft.base + draft.assigned.len() as u64;
        self.keys
            .stage(submit, self.active.length, draft.assigned.len());
        draft
            .origins
            .extend((0..submit.records.len()).map(|record| (draft.requests.len(), record)));
        draft.size += body_size(submit).unwrap() - codec::BODY_HEADER_LEN;
        for (i, (record, revision)) in submit.records.iter().zip(revisions).enumerate() {
            if !self.active.tails.contains_key(&record.stream)
                && !draft.tails.contains_key(&record.stream)
            {
                draft.footer_growth += footer_entry(&record.stream);
            }
            draft.tails.insert(
                record.stream.clone(),
                Tail {
                    revision: revision.revision,
                    offset: first + i as u64,
                    position: self.active.length,
                },
            );
        }
        draft
            .assigned
            .extend(revisions.iter().map(|slot| slot.revision));
        #[cfg(test)]
        crate::pipeline::request(submit, crate::pipeline::READY);
        draft.requests.push(request);
    }
}

/// A dequeued seal marker ends the gather, because nothing is queued behind it.
fn dequeued(message: Option<Message>, seal: &mut Option<mpsc::Sender<()>>) -> Option<Request> {
    match message? {
        Message::Append(request) => Some(request),
        Message::Seal(reply) => {
            *seal = Some(reply);
            None
        }
    }
}

/// Marks the writer poisoned when its thread panics (api.md, admission step 3).
struct PoisonOnPanic<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for PoisonOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

pub(super) fn run(
    view: Arc<View>,
    poisoned: Arc<std::sync::atomic::AtomicBool>,
    receiver: mpsc::Receiver<Message>,
    mut writer: Writer,
    options: Arc<Options>,
) {
    // A local, so that unwinding drops it before `receiver`: admission sees the flag before it
    // can see the disconnected queue.
    let _panic = PoisonOnPanic(&poisoned);
    let mut deferred = None;
    let mut seal: Option<mpsc::Sender<()>> = None;
    loop {
        let first = match deferred.take() {
            Some(request) => request,
            None => {
                if let Some(reply) = seal.take() {
                    if writer.seal(&view).is_ok() {
                        let _ = reply.send(());
                    } else {
                        poisoned.store(true, std::sync::atomic::Ordering::Release);
                    }
                    return;
                }
                let Ok(message) = receiver.recv() else {
                    return;
                };
                let Some(request) = dequeued(Some(message), &mut seal) else {
                    continue;
                };
                request
            }
        };
        #[cfg(test)]
        crate::pipeline::request(&first.submit, crate::pipeline::DEQUEUE);
        let mut draft = Draft::new(view.watermark());
        // When the writer first saw this group's linger over.
        let mut gathered = None;
        let mut next = Some(first);
        while let Some(request) = next.take() {
            #[cfg(test)]
            crate::pipeline::request(&request.submit, crate::pipeline::CLASSIFY);
            match writer
                .keys
                .classify(&view, &request.submit, &draft.staged(writer.active.length))
            {
                Classification::New => {
                    match revisions(&request.submit, &draft.tails, &view.published) {
                        Err(error) => {
                            request.answer(Err(error));
                        }
                        Ok(revisions) => {
                            let added =
                                body_size(&request.submit).unwrap() - codec::BODY_HEADER_LEN;
                            let fits = writer.fits(&draft, &request.submit);
                            if draft.size + added > options.group_byte_cap
                                || (!fits && !draft.requests.is_empty())
                            {
                                deferred = Some(request);
                                break;
                            }
                            if draft
                                .base
                                .checked_add(draft.assigned.len() as u64)
                                .and_then(|next| {
                                    next.checked_add(request.submit.records.len() as u64)
                                })
                                .is_none()
                            {
                                request.answer(Err(CommitError::Exhausted));
                            } else {
                                // The roll waits for the group's first new submit, so that a group
                                // that appends nothing writes and flushes nothing.
                                if draft.requests.is_empty()
                                    && writer.rolls_before(draft.base, fits, &options)
                                {
                                    if writer.roll(&view, draft.base).is_err() {
                                        poison(&poisoned, draft, Some(request), receiver);
                                        return;
                                    }
                                    debug_assert!(
                                        writer.fits(&draft, &request.submit),
                                        "a submit under the group cap fits an empty segment's closing frames"
                                    );
                                }
                                writer.stage(&mut draft, request, &revisions);
                            }
                        }
                    }
                }
                Classification::DurableRetry(records) => {
                    request.answer(Ok(Committed {
                        records,
                        deduplicated: true,
                    }));
                }
                Classification::PendingRetry(positions) => {
                    draft.waiters.push(RetryWaiter { request, positions });
                }
                Classification::Rejected(error) => {
                    request.answer(Err(error));
                }
                Classification::LookupFailure => {
                    draft.requests.push(request);
                    poison(&poisoned, draft, deferred.take(), receiver);
                    return;
                }
            }
            next = match draft.requests.first() {
                _ if seal.is_some() => None,
                // No new submit has joined yet, so no linger runs.
                None => dequeued(receiver.try_recv().ok(), &mut seal),
                Some(first) => {
                    // The linger runs from the admission of the group's first submit, so the time
                    // that submit spent queued behind the previous flush counts (ADR 0046).
                    let remaining = options.linger.saturating_sub(first.admitted.elapsed());
                    if remaining.is_zero() {
                        // The group takes the submits admitted before the writer first saw the
                        // linger over. Without that limit, steady arrivals keep one group
                        // gathering until the byte cap.
                        let limit = *gathered.get_or_insert_with(Instant::now);
                        match dequeued(receiver.try_recv().ok(), &mut seal) {
                            Some(late) if late.admitted > limit => {
                                deferred = Some(late);
                                None
                            }
                            queued => queued,
                        }
                    } else {
                        dequeued(receiver.recv_timeout(remaining).ok(), &mut seal)
                    }
                }
            };
            #[cfg(test)]
            if let Some(request) = &next {
                crate::pipeline::request(&request.submit, crate::pipeline::DEQUEUE);
            }
        }
        if draft.requests.is_empty() {
            continue;
        }
        if writer.commit(&view, &mut draft).is_err() {
            poison(&poisoned, draft, deferred.take(), receiver);
            return;
        }
    }
}

impl Writer {
    /// A failed write or flush leaves every request of the draft unanswered.
    fn commit(&mut self, view: &View, draft: &mut Draft) -> io::Result<()> {
        let base = draft.base;
        #[cfg(test)]
        let mut timeline = crate::pipeline::Group::new(base, draft.requests.len());
        let entries = draft
            .requests
            .iter()
            .flat_map(|r| &r.submit.records)
            .zip(&draft.assigned)
            .map(|(r, revision)| codec::Entry {
                stream: &r.stream,
                key: r.key.as_deref(),
                payload: &r.payload,
                revision: *revision,
            })
            .collect();
        let group = codec::Group {
            epoch: self.epoch,
            base_offset: base,
            entries,
        };
        let bytes = codec::encode_group(&view.log_id, &group).expect("validated group");
        let length = bytes.len();
        let segment = segment_path(&view.dir, self.active.base);
        #[cfg(test)]
        timeline.mark(1);
        let result = call(
            view.storage.as_ref(),
            &segment,
            Purpose::Group,
            Op::Append(bytes),
        )
        .and_then(|_| {
            #[cfg(test)]
            timeline.mark(2);
            call(
                view.storage.as_ref(),
                &segment,
                Purpose::Group,
                Op::SyncData,
            )
        });
        #[cfg(test)]
        timeline.mark(3);
        result?;
        let span = GroupSpan::of(self.active.length, length, &group);
        self.active.length += length as u64;
        self.active.tails.append(&mut draft.tails);
        self.active.footer_body += std::mem::take(&mut draft.footer_growth);
        view.published.lock().unwrap().publish(span, &group);
        #[cfg(test)]
        timeline.mark(4);
        let mut index = 0;
        let mut slots = Vec::with_capacity(draft.requests.len() + draft.waiters.len());
        for request in std::mem::take(&mut draft.requests) {
            #[cfg(test)]
            crate::pipeline::group_id(&request.submit, base);
            let records = (0..request.submit.records.len())
                .map(|_| {
                    let p = Position {
                        offset: base + index as u64,
                        revision: draft.assigned[index],
                    };
                    index += 1;
                    p
                })
                .collect();
            slots.push(request.store(Ok(Committed {
                records,
                deduplicated: false,
            })));
        }
        for waiter in std::mem::take(&mut draft.waiters) {
            slots.push(waiter.request.store(Ok(Committed {
                records: waiter.positions,
                deduplicated: true,
            })));
        }
        #[cfg(test)]
        {
            *self.memory.lock().unwrap() = self.keys.memory();
        }
        reply::Relay::start(slots);
        #[cfg(test)]
        timeline.finish();
        Ok(())
    }
}

fn poison(
    poisoned: &std::sync::atomic::AtomicBool,
    draft: Draft,
    current: Option<Request>,
    receiver: mpsc::Receiver<Message>,
) {
    poisoned.store(true, std::sync::atomic::Ordering::Release);
    for request in draft.requests {
        request.answer(Err(CommitError::Indeterminate));
    }
    for waiter in draft.waiters {
        waiter.request.answer(Err(CommitError::Indeterminate));
    }
    if let Some(request) = current {
        request.answer(Err(CommitError::Poisoned));
    }
    for message in receiver {
        if let Message::Append(request) = message {
            request.answer(Err(CommitError::Poisoned));
        }
    }
}
