//! Request scheduling for the single disk owner.
use super::*;
impl Store {
    pub(in crate::recovery) fn run(mut self, receiver: mpsc::Receiver<Command>) {
        loop {
            let received = match self.next_wait(crate::desktop::events::now_ms()) {
                Some(wait) => receiver.recv_timeout(wait),
                None => receiver
                    .recv()
                    .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
            };
            let command = match received {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => None,
            };
            if let Some(command) = command {
                if let Command::Stop(reply) = command {
                    let ids: Vec<_> = self.sessions.keys().cloned().collect();
                    for id in ids {
                        self.sessions[&id]
                            .read_cancel
                            .store(true, Ordering::Release);
                        if self.sessions[&id]
                            .manifest
                            .as_ref()
                            .is_some_and(|manifest| {
                                manifest.snapshot.capture == CaptureState::Recording
                            })
                        {
                            let _ = self.checkpoint(&id, None);
                            self.fault(&id, Failure::Interrupted);
                        }
                    }
                    // Release the actual OS lock before acknowledging shutdown.
                    drop(self);
                    let _ = reply.send(Ok(()));
                    return;
                }
                self.handle(command);
            }
            self.maintenance();
        }
    }
    fn handle(&mut self, command: Command) {
        match command {
            Command::Begin {
                id,
                selection,
                health,
            } => {
                if let Err(error) = self.begin(id, selection, health.clone()) {
                    health
                        .fault
                        .store(encode_fault(classify(&error)), Ordering::Release);
                }
            }
            Command::Audio { id, start, samples } => {
                if let Err(error) = self.audio(&id, start, samples) {
                    self.fault(&id, classify(&error));
                }
                if let Some(health) = self
                    .sessions
                    .get(&id)
                    .and_then(|session| session.health.as_ref())
                {
                    health.pending.lock().unwrap().pop_front();
                }
            }
            Command::Gap { id, missing } => {
                if let Err(error) = self.gap(&id, missing) {
                    self.fault(&id, classify(&error));
                }
            }
            Command::Seal { id, samples } => {
                if let Err(error) = self.checkpoint(&id, Some(samples)) {
                    self.fault(&id, classify(&error));
                }
            }
            Command::Text {
                id,
                text,
                raw,
                intent,
                reply,
            } => {
                let result = self
                    .text(&id, text, raw, intent, None)
                    .map_err(|error| classify(&error));
                let _ = reply.send(result);
            }
            Command::Outcome {
                id,
                delivery,
                reply,
            } => {
                let result = (|| {
                    let session = self.sessions.get_mut(&id).ok_or(Failure::Storage)?;
                    if session.blocked {
                        return Err(Failure::Storage);
                    }
                    let manifest = session.manifest.as_mut().ok_or(Failure::Storage)?;
                    manifest.snapshot.resolved_ms = None;
                    manifest.snapshot.delivery = delivery;
                    if matches!(delivery, Delivery::ReportedSuccess | Delivery::Suppressed)
                        && manifest.snapshot.error.is_none()
                    {
                        manifest.snapshot.resolved_ms = Some(crate::desktop::events::now_ms());
                    }
                    self.persist(&id).map_err(|error| classify(&error))
                })();
                let _ = reply.send(result);
            }
            Command::ReplayStart { id, reply } => {
                let result = (|| {
                    let session = self.sessions.get_mut(&id).ok_or(Failure::Storage)?;
                    if session.blocked || session.readers.load(Ordering::Acquire) != 0 {
                        return Err(Failure::Storage);
                    }
                    let manifest = session.manifest.as_mut().ok_or(Failure::Storage)?;
                    if manifest.snapshot.capture == CaptureState::Recording
                        || manifest.snapshot.retrying
                    {
                        return Err(Failure::Storage);
                    }
                    manifest.snapshot.retrying = true;
                    manifest.snapshot.resolved_ms = None;
                    manifest.snapshot.user_resolved = false;
                    self.persist(&id).map_err(|error| classify(&error))?;
                    self.reader(&id).map_err(|error| classify(&error))
                })();
                let _ = reply.send(result);
            }
            Command::ReplayFinish {
                reader,
                selection,
                result,
                reply,
            } => {
                let id = reader.id.clone();
                let result = (|| {
                    let session = self.sessions.get_mut(&id).ok_or(Failure::Storage)?;
                    if session.blocked {
                        return Err(Failure::Storage);
                    }
                    let manifest = session.manifest.as_mut().ok_or(Failure::Storage)?;
                    if manifest.snapshot.revision != reader.revision {
                        return Err(Failure::Storage);
                    }
                    manifest.snapshot.retrying = false;
                    match result {
                        Ok(text) if reader.complete => {
                            manifest.snapshot.capture = CaptureState::Sealed;
                            manifest.snapshot.samples = reader.at;
                            manifest.snapshot.gap_count = reader.gaps;
                            self.text(&id, text, true, false, Some(selection))
                                .map_err(|error| classify(&error))
                        }
                        result => {
                            manifest.snapshot.error =
                                Some(result.err().unwrap_or(Failure::Storage));
                            self.persist(&id).map_err(|error| classify(&error))
                        }
                    }
                })();
                let _ = reply.send(result);
            }
            Command::List(reply) => {
                let _ = reply.send(Ok(self
                    .sessions
                    .iter()
                    .map(|(id, session)| Entry {
                        id: id.clone(),
                        snapshot: session
                            .manifest
                            .as_ref()
                            .map(|manifest| manifest.snapshot.clone()),
                        bytes: session.charged,
                    })
                    .collect()));
            }
            Command::ReadText { id, raw, reply } => {
                let _ = reply.send(self.read_text(&id, raw).map_err(|error| classify(&error)));
            }
            Command::Reader { id, reply } => {
                let _ = reply.send(self.reader(&id).map_err(|error| classify(&error)));
            }
            Command::Resolve { id, keep, reply } => {
                let result = (|| {
                    let session = self.sessions.get_mut(&id).ok_or(Failure::Storage)?;
                    if session.readers.load(Ordering::Acquire) != 0 {
                        return Err(Failure::Storage);
                    }
                    let manifest = session.manifest.as_mut().ok_or(Failure::Storage)?;
                    if manifest.snapshot.capture == CaptureState::Recording {
                        return Err(Failure::Storage);
                    }
                    if session.blocked {
                        return Err(Failure::Storage);
                    }
                    manifest.snapshot.keep = keep;
                    manifest.snapshot.user_resolved = true;
                    manifest.snapshot.resolved_ms = Some(crate::desktop::events::now_ms());
                    self.persist(&id).map_err(|error| classify(&error))
                })();
                let _ = reply.send(result);
            }
            Command::Delete { id, reply } => {
                let _ = reply.send(self.delete(&id).map_err(|error| classify(&error)));
            }
            Command::Stop(_) => unreachable!("shutdown is handled before dispatch"),
        }
    }
}
