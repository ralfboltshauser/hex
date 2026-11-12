use super::*;

/// Sequential bounded replay. A live reader pins the session against deletion;
/// frames outside the sealed extent are never returned.
pub struct AudioReader {
    pub(super) id: SessionId,
    pub(super) revision: u64,
    pub(super) selection: TranscriptionSelection,
    pub(super) complete: bool,
    pub(super) directory: Directory,
    pub(super) readers: Arc<AtomicU64>,
    pub(super) read_cancel: Arc<AtomicBool>,
    pub(super) limit: u64,
    pub(super) acknowledged: u64,
    pub(super) salvage: bool,
    pub(super) gaps: u64,
    pub(super) at: u64,
    pub(super) sequence: u32,
    pub(super) segment: Option<(u64, File)>,
}
impl AudioReader {
    pub(in crate::recovery) fn selection(&self) -> &TranscriptionSelection {
        &self.selection
    }
    pub fn next(&mut self) -> io::Result<Option<Vec<f32>>> {
        if self.read_cancel.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "audio read cancelled",
            ));
        }
        if self.at >= self.limit {
            self.complete = true;
            return Ok(None);
        }
        let segment = self.at / SEGMENT_SAMPLES;
        if self
            .segment
            .as_ref()
            .is_none_or(|(current, _)| *current != segment)
        {
            let file = match self.directory.file(&audio_name(segment), false) {
                Ok(file) => file,
                Err(error)
                    if error.kind() == io::ErrorKind::NotFound
                        && self.salvage
                        && self.at >= self.acknowledged =>
                {
                    self.complete = true;
                    return Ok(None);
                }
                Err(error) => return Err(error),
            };
            self.segment = Some((segment, file));
        }
        let frame = format::read_frame(&mut self.segment.as_mut().unwrap().1);
        let mut frame = match frame {
            Ok(Some(frame)) if frame.start == self.at && frame.sequence == self.sequence => frame,
            Ok(None) | Err(_) if self.salvage && self.at >= self.acknowledged => {
                self.complete = true;
                return Ok(None);
            }
            Ok(_) => return Err(io::Error::other("missing or discontinuous committed audio")),
            Err(error) => return Err(error),
        };
        if frame.missing.is_some() {
            self.gaps = self.gaps.saturating_add(1);
        }
        frame
            .samples
            .truncate((self.limit - self.at).min(frame.samples.len() as u64) as usize);
        self.at += frame.samples.len() as u64;
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("audio sequence exhausted"))?;
        Ok(Some(frame.samples))
    }
}
impl Drop for AudioReader {
    fn drop(&mut self) {
        self.readers.fetch_sub(1, Ordering::AcqRel);
    }
}
