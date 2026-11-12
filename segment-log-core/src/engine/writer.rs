use super::*;
use keys::Classification;

pub(super) struct Writer {
    pub epoch: u64,
    pub keys: Keys,
    pub lookup: Vec<u8>,
    pub active: Active,
}

/// The segment that takes new frames. Only the writer thread appends to it.
pub(super) struct Active {
    pub base: u64,
    pub length: u64,
}

impl Writer {
    /// Starts a new active segment at `next`, the next log offset, if the active one has
    /// reached the roll size.
    fn roll(&mut self, view: &View, next: u64, options: &Options) -> io::Result<()> {
        // A segment with no record cannot roll: its successor would take its name.
        if self.active.length < options.roll_bytes || next == self.active.base {
            return Ok(());
        }
        let header = codec::Header {
            log_id: *view.log_id.bytes(),
            base_offset: next,
            epoch: self.epoch,
        };
        create_segment(view.storage.as_ref(), &view.dir, Purpose::Roll, &header)?;
        view.published.lock().unwrap().roll();
        self.active = Active {
            base: next,
            length: codec::HEADER_LEN as u64,
        };
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
    pending: &BTreeMap<Vec<u8>, u64>,
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
            .copied()
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

pub(super) fn run(
    view: Arc<View>,
    poisoned: Arc<std::sync::atomic::AtomicBool>,
    receiver: mpsc::Receiver<Request>,
    mut writer: Writer,
    options: Arc<Options>,
) {
    let mut deferred = None;
    loop {
        let first = match deferred.take().or_else(|| receiver.recv().ok()) {
            Some(r) => r,
            None => return,
        };
        #[cfg(test)]
        crate::pipeline::request(&first.submit, crate::pipeline::DEQUEUE);
        let base = view.watermark();
        // Copying every durable head here would make each group cost grow with the stream count.
        let mut heads = BTreeMap::new();
        let mut requests = Vec::new();
        let mut assigned = Vec::new();
        let mut origins = Vec::new();
        let mut waiters = Vec::new();
        let mut size = codec::BODY_HEADER_LEN;
        // When the writer first saw this group's linger over.
        let mut gathered = None;
        let mut next = Some(first);
        while let Some(request) = next.take() {
            #[cfg(test)]
            crate::pipeline::request(&request.submit, crate::pipeline::CLASSIFY);
            match writer.keys.classify(
                &view,
                &request.submit,
                &mut writer.lookup,
                &keys::Staged {
                    base,
                    requests: &requests,
                    origins: &origins,
                },
            ) {
                Classification::New => match revisions(&request.submit, &heads, &view.published) {
                    Err(error) => {
                        request.answer(Err(error));
                    }
                    Ok(revisions) => {
                        let added = body_size(&request.submit).unwrap() - codec::BODY_HEADER_LEN;
                        if size + added > options.group_byte_cap {
                            deferred = Some(request);
                            break;
                        }
                        if base
                            .checked_add(assigned.len() as u64)
                            .and_then(|next| next.checked_add(request.submit.records.len() as u64))
                            .is_none()
                        {
                            request.answer(Err(CommitError::Exhausted));
                        } else {
                            // The roll waits for the group's first new submit, so that a group
                            // that appends nothing writes and flushes nothing.
                            if requests.is_empty() && writer.roll(&view, base, &options).is_err() {
                                poison(&poisoned, requests, waiters, Some(request), receiver);
                                return;
                            }
                            writer.keys.stage(
                                &request.submit,
                                base + assigned.len() as u64,
                                &revisions,
                            );
                            origins.extend(
                                (0..request.submit.records.len())
                                    .map(|record| (requests.len(), record)),
                            );
                            size += added;
                            for (record, revision) in request.submit.records.iter().zip(&revisions)
                            {
                                heads.insert(record.stream.clone(), revision.revision);
                            }
                            assigned.extend(revisions.iter().map(|slot| slot.revision));
                            #[cfg(test)]
                            crate::pipeline::request(&request.submit, crate::pipeline::READY);
                            requests.push(request);
                        }
                    }
                },
                Classification::DurableRetry(records) => {
                    request.answer(Ok(Committed {
                        records,
                        deduplicated: true,
                    }));
                }
                Classification::PendingRetry(positions) => {
                    waiters.push(RetryWaiter { request, positions });
                }
                Classification::Rejected(error) => {
                    request.answer(Err(error));
                }
                Classification::LookupFailure => {
                    requests.push(request);
                    poison(&poisoned, requests, waiters, deferred.take(), receiver);
                    return;
                }
            }
            next = match requests.first() {
                // No new submit has joined yet, so no linger runs.
                None => receiver.try_recv().ok(),
                Some(first) => {
                    // The linger runs from the admission of the group's first submit, so the time
                    // that submit spent queued behind the previous flush counts (ADR 0046).
                    let remaining = options.linger.saturating_sub(first.admitted.elapsed());
                    if remaining.is_zero() {
                        // The group takes the submits admitted before the writer first saw the
                        // linger over. Without that limit, steady arrivals keep one group
                        // gathering until the byte cap.
                        let limit = *gathered.get_or_insert_with(Instant::now);
                        match receiver.try_recv().ok() {
                            Some(late) if late.admitted > limit => {
                                deferred = Some(late);
                                None
                            }
                            queued => queued,
                        }
                    } else {
                        receiver.recv_timeout(remaining).ok()
                    }
                }
            };
            #[cfg(test)]
            if let Some(request) = &next {
                crate::pipeline::request(&request.submit, crate::pipeline::DEQUEUE);
            }
        }
        if requests.is_empty() {
            continue;
        }
        #[cfg(test)]
        let mut timeline = crate::pipeline::Group::new(base, requests.len());
        let entries = requests
            .iter()
            .flat_map(|r| &r.submit.records)
            .zip(&assigned)
            .map(|(r, revision)| codec::Entry {
                stream: &r.stream,
                key: r.key.as_deref(),
                payload: &r.payload,
                revision: *revision,
            })
            .collect();
        let group = codec::Group {
            epoch: writer.epoch,
            base_offset: base,
            entries,
        };
        let bytes = codec::encode_group(&view.log_id, &group).expect("validated group");
        let length = bytes.len();
        let segment = segment_path(&view.dir, writer.active.base);
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
        if result.is_err() {
            poison(&poisoned, requests, waiters, deferred.take(), receiver);
            return;
        }
        let span = GroupSpan::of(writer.active.length, length, &group);
        writer.active.length += length as u64;
        view.published.lock().unwrap().publish(span, &group);
        #[cfg(test)]
        timeline.mark(4);
        let mut index = 0;
        let mut slots = Vec::with_capacity(requests.len() + waiters.len());
        for request in requests {
            #[cfg(test)]
            crate::pipeline::group_id(&request.submit, base);
            let records = (0..request.submit.records.len())
                .map(|_| {
                    let p = Position {
                        offset: base + index as u64,
                        revision: assigned[index],
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
        for waiter in waiters {
            slots.push(waiter.request.store(Ok(Committed {
                records: waiter.positions,
                deduplicated: true,
            })));
        }
        reply::Relay::start(slots);
        #[cfg(test)]
        timeline.finish();
    }
}

fn poison(
    poisoned: &std::sync::atomic::AtomicBool,
    requests: Vec<Request>,
    waiters: Vec<RetryWaiter>,
    deferred: Option<Request>,
    receiver: mpsc::Receiver<Request>,
) {
    poisoned.store(true, std::sync::atomic::Ordering::Release);
    for request in requests {
        request.answer(Err(CommitError::Indeterminate));
    }
    for waiter in waiters {
        waiter.request.answer(Err(CommitError::Indeterminate));
    }
    if let Some(request) = deferred {
        request.answer(Err(CommitError::Poisoned));
    }
    for request in receiver {
        request.answer(Err(CommitError::Poisoned));
    }
}
