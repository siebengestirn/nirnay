use std::fs::{
    File, //
    OpenOptions,
};
use std::io::{
    BufRead, //
    BufReader,
    Write,
};
use std::path::Path;

use fs2::FileExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::{ArchiveError, RawFrame};

/// Single-writer, newline-delimited raw-frame journal.
///
/// Each append calls `sync_data`; this favours durability over throughput.
/// The OS file lock prevents two processes from concurrently appending. A
/// future recorder may add bounded group commit, but must preserve the recovery
/// contract and measure its durability/latency trade-off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriterState {
    Healthy,
    Poisoned,
}

/// Failure point for the private test I/O seam. It is not part of the public
/// journal API.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FailPoint {
    PartialRecordWrite,
    RecordWrite,
    NewlineWrite,
    Sync,
}

/// Private I/O boundary. Production uses a real `File`; tests use `Failing`,
/// which performs the real OS call before returning the injected `io::Error`.
/// The writer therefore observes the same error path as an actual write or
/// `sync_data` failure without a public generic storage abstraction.
enum JournalIo {
    File(File),
    #[cfg(test)]
    Failing {
        file: File,
        point: Option<FailPoint>,
    },
}

impl JournalIo {
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        match self {
            Self::File(file) => file.write_all(bytes),
            #[cfg(test)]
            Self::Failing { file, point } => match point {
                Some(FailPoint::PartialRecordWrite) => {
                    let split = bytes.len().saturating_sub(1);
                    file.write_all(&bytes[..split])?;
                    *point = None;
                    Err(std::io::Error::other(
                        "injected partial record write failure",
                    ))
                }
                Some(FailPoint::RecordWrite) => {
                    *point = None;
                    Err(std::io::Error::other("injected record write failure"))
                }
                Some(FailPoint::NewlineWrite) if bytes == b"\n" => {
                    file.write_all(bytes)?;
                    *point = None;
                    Err(std::io::Error::other("injected newline write failure"))
                }
                _ => file.write_all(bytes),
            },
        }
    }

    fn sync_data(&mut self) -> std::io::Result<()> {
        match self {
            Self::File(file) => file.sync_data(),
            #[cfg(test)]
            Self::Failing { file, point } => {
                file.sync_data()?;
                if matches!(point, Some(FailPoint::Sync)) {
                    *point = None;
                    return Err(std::io::Error::other("injected sync_data failure"));
                }
                Ok(())
            }
        }
    }
}

pub struct JournalWriter {
    io: JournalIo,
    state: WriterState,
}

fn uncertain_outcome(context: &'static str, source: std::io::Error) -> ArchiveError {
    ArchiveError::CommitOutcomeUnknown { context, source }
}

fn create_journal_file(path: &Path) -> Result<File, ArchiveError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options.open(path)?;
    file.try_lock_exclusive()?;
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(file)
}

impl JournalWriter {
    pub fn create_new(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        Ok(Self {
            io: JournalIo::File(create_journal_file(path.as_ref())?),
            state: WriterState::Healthy,
        })
    }

    /// Test-only constructor using the private I/O seam. It performs real OS
    /// writes and then returns an `io::Error` at the selected point.
    #[cfg(test)]
    fn create_new_with_fail_point(
        path: impl AsRef<Path>,
        point: FailPoint,
    ) -> Result<Self, ArchiveError> {
        Ok(Self {
            io: JournalIo::Failing {
                file: create_journal_file(path.as_ref())?,
                point: Some(point),
            },
            state: WriterState::Healthy,
        })
    }

    pub fn open_append(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        let path = path.as_ref();
        let file = OpenOptions::new().append(true).open(path)?;
        file.try_lock_exclusive()?;
        // Never append behind a torn or corrupted tail. Validation is streamed
        // while the exclusive lock is held, so reopening a large journal does
        // not require loading every historical frame into memory.
        let _ = verify_journal(path)?;
        Ok(Self {
            io: JournalIo::File(file),
            state: WriterState::Healthy,
        })
    }

    /// Append one validated frame.
    ///
    /// Validation and serialization happen before any file I/O, so an invalid
    /// frame leaves the writer healthy. Once any write or sync fails the commit
    /// outcome is unknown: the writer is poisoned and every later append is
    /// refused before touching the file. Recovery requires dropping and
    /// reopening the journal, which verifies the committed tail.
    pub fn append(&mut self, frame: &RawFrame) -> Result<(), ArchiveError> {
        if self.state == WriterState::Poisoned {
            return Err(ArchiveError::Poisoned);
        }
        let line = frame.to_json_line()?;
        if let Err(error) = self.io.write_all(line.as_bytes()) {
            self.state = WriterState::Poisoned;
            return Err(uncertain_outcome("record write", error));
        }
        if let Err(error) = self.io.write_all(b"\n") {
            self.state = WriterState::Poisoned;
            return Err(uncertain_outcome("commit marker write", error));
        }
        if let Err(error) = self.io.sync_data() {
            self.state = WriterState::Poisoned;
            return Err(uncertain_outcome("sync_data", error));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalScan {
    pub frames: Vec<RawFrame>,
    pub bytes_scanned: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalSummary {
    pub frame_count: u64,
    pub bytes_scanned: u64,
}

/// Visit and validate each journal record using record-by-record streaming.
/// The callback determines whether callers retain frames or only count them.
fn visit_journal(
    path: impl AsRef<Path>,
    mut visit: impl FnMut(RawFrame) -> Result<(), ArchiveError>,
) -> Result<JournalSummary, ArchiveError> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut bytes_scanned = 0_u64;
    let mut frame_count = 0_u64;
    let mut line_number = 0_u64;

    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line)?;
        if read == 0 {
            break;
        }
        line_number = line_number.saturating_add(1);
        bytes_scanned = bytes_scanned.saturating_add(read as u64);
        if !line.ends_with(b"\n") {
            return Err(ArchiveError::TornTail);
        }
        line.pop();
        if line.is_empty() {
            return Err(ArchiveError::MalformedRecord(format!(
                "line {line_number} is empty"
            )));
        }
        let text = std::str::from_utf8(&line).map_err(|error| {
            ArchiveError::MalformedRecord(format!("line {line_number} is not UTF-8: {error}"))
        })?;
        let frame = RawFrame::from_json_line(text)?;
        visit(frame)?;
        frame_count = frame_count.saturating_add(1);
    }

    Ok(JournalSummary {
        frame_count,
        bytes_scanned,
    })
}

/// Validate the entire journal without retaining decoded frames in memory.
/// This is the recovery/open-append path for long-running recordings.
pub fn verify_journal(path: impl AsRef<Path>) -> Result<JournalSummary, ArchiveError> {
    visit_journal(path, |_| Ok(()))
}

/// Scan and return all decoded frames. Use [`verify_journal`] for a large
/// archive when only integrity and counts are required.
pub fn scan_journal(path: impl AsRef<Path>) -> Result<JournalScan, ArchiveError> {
    let mut frames = Vec::new();
    let summary = visit_journal(path, |frame| {
        frames.push(frame);
        Ok(())
    })?;
    Ok(JournalScan {
        frames,
        bytes_scanned: summary.bytes_scanned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(sequence: u64) -> RawFrame {
        let payload = format!("payload-{sequence}");
        RawFrame::new(
            "test-feed",
            "TEST:ABC",
            sequence as i64,
            sequence,
            None,
            Some(sequence),
            "synthetic-fixture",
            payload.as_bytes(),
        )
        .unwrap()
    }

    #[test]
    fn append_and_scan_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer = JournalWriter::create_new(&path).unwrap();
        writer.append(&frame(1)).unwrap();
        writer.append(&frame(2)).unwrap();
        drop(writer);

        let scan = scan_journal(&path).unwrap();
        assert_eq!(scan.frames, vec![frame(1), frame(2)]);
        assert!(scan.bytes_scanned > 0);
        assert_eq!(
            verify_journal(&path).unwrap(),
            JournalSummary {
                frame_count: 2,
                bytes_scanned: scan.bytes_scanned,
            }
        );
    }

    #[test]
    fn torn_tail_is_reported_and_not_repaired() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        std::fs::write(&path, b"{\"partial\":").unwrap();

        assert!(matches!(scan_journal(&path), Err(ArchiveError::TornTail)));
        assert!(matches!(verify_journal(&path), Err(ArchiveError::TornTail)));
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"partial\":");
    }

    #[test]
    fn tampered_record_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let line = frame(1).to_json_line().unwrap();
        let tampered = line.replace("cGF5bG9hZC0x", "cGF5bG9hZC0y");
        std::fs::write(&path, format!("{tampered}\n")).unwrap();

        assert!(matches!(
            scan_journal(&path),
            Err(ArchiveError::PayloadHashMismatch)
        ));
    }

    #[test]
    fn append_refuses_to_extend_a_torn_journal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        std::fs::write(&path, b"{\"partial\":").unwrap();
        assert!(matches!(
            JournalWriter::open_append(&path),
            Err(ArchiveError::TornTail)
        ));
    }

    #[test]
    fn a_second_writer_cannot_acquire_the_same_journal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let writer = JournalWriter::create_new(&path).unwrap();
        assert!(JournalWriter::open_append(&path).is_err());
        drop(writer);
        assert!(JournalWriter::open_append(&path).is_ok());
    }

    #[test]
    fn an_empty_file_is_a_valid_empty_journal() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("empty.jsonl");
        std::fs::write(&path, b"").unwrap();
        assert_eq!(
            verify_journal(&path).unwrap(),
            JournalSummary {
                frame_count: 0,
                bytes_scanned: 0,
            }
        );
    }

    fn unknown_context(error: &ArchiveError) -> &'static str {
        match error {
            ArchiveError::CommitOutcomeUnknown { context, .. } => context,
            other => panic!("expected uncertain commit outcome, got {other:?}"),
        }
    }

    #[test]
    fn partial_record_write_is_uncertain_and_poisons_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer =
            JournalWriter::create_new_with_fail_point(&path, FailPoint::PartialRecordWrite)
                .unwrap();

        let error = writer.append(&frame(1)).unwrap_err();
        assert_eq!(
            unknown_context(&error),
            "record write",
            "partial write context"
        );
        assert!(
            matches!(
                &error,
                ArchiveError::CommitOutcomeUnknown { source, .. }
                    if source.to_string().contains("partial record write")
            ),
            "partial write reports uncertain outcome with original error"
        );
        assert_eq!(
            writer.state,
            WriterState::Poisoned,
            "partial write poisons writer"
        );
        let bytes_after_failure = std::fs::read(&path).unwrap();
        assert!(
            !bytes_after_failure.is_empty(),
            "partial record bytes must remain on disk"
        );

        assert!(
            matches!(writer.append(&frame(2)), Err(ArchiveError::Poisoned)),
            "poisoned writer refuses append"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes_after_failure,
            "poisoned writer must not mutate file"
        );
    }

    #[test]
    fn newline_write_failure_is_uncertain_and_poisons_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer =
            JournalWriter::create_new_with_fail_point(&path, FailPoint::NewlineWrite).unwrap();

        let error = writer.append(&frame(1)).unwrap_err();
        assert_eq!(
            unknown_context(&error),
            "commit marker write",
            "newline failure context"
        );
        assert_eq!(
            writer.state,
            WriterState::Poisoned,
            "newline failure poisons writer"
        );
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.ends_with(b"\n"),
            "a failed newline write may still leave bytes on disk"
        );
        assert!(
            matches!(writer.append(&frame(2)), Err(ArchiveError::Poisoned)),
            "poisoned writer refuses append"
        );
    }

    #[test]
    fn sync_data_failure_is_uncertain_and_poisons_writer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer = JournalWriter::create_new_with_fail_point(&path, FailPoint::Sync).unwrap();

        let error = writer.append(&frame(1)).unwrap_err();
        assert_eq!(unknown_context(&error), "sync_data", "sync failure context");
        assert_eq!(
            writer.state,
            WriterState::Poisoned,
            "sync failure poisons writer"
        );
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.ends_with(b"\n"),
            "bytes written before failed sync remain"
        );
        assert!(
            matches!(writer.append(&frame(2)), Err(ArchiveError::Poisoned)),
            "poisoned writer refuses append"
        );
    }

    #[test]
    fn append_after_poisoning_refuses_before_mutating_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer =
            JournalWriter::create_new_with_fail_point(&path, FailPoint::RecordWrite).unwrap();
        assert!(
            matches!(
                writer.append(&frame(1)),
                Err(ArchiveError::CommitOutcomeUnknown { .. })
            ),
            "failed dispatch reports uncertain outcome"
        );
        let bytes = std::fs::read(&path).unwrap();
        assert!(
            matches!(writer.append(&frame(2)), Err(ArchiveError::Poisoned)),
            "poisoned writer refuses append"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "poisoned append leaves file unchanged"
        );
    }

    #[test]
    fn invalid_frame_before_writing_leaves_writer_healthy() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer = JournalWriter::create_new(&path).unwrap();
        let mut invalid = frame(1);
        invalid.payload_sha256 =
            "0000000000000000000000000000000000000000000000000000000000000000".to_owned();

        assert!(
            matches!(
                writer.append(&invalid),
                Err(ArchiveError::PayloadHashMismatch)
            ),
            "invalid frame is rejected before I/O"
        );
        assert_eq!(
            writer.state,
            WriterState::Healthy,
            "invalid frame must not poison writer"
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"",
            "invalid frame performs no I/O"
        );
        writer.append(&frame(1)).unwrap();
        assert_eq!(
            verify_journal(&path).unwrap().frame_count,
            1,
            "valid frame after invalid one appends"
        );
    }

    #[test]
    fn successful_append_writes_a_complete_synced_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("raw.jsonl");
        let mut writer = JournalWriter::create_new(&path).unwrap();
        writer.append(&frame(7)).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            bytes.ends_with(b"\n"),
            "successful append writes commit marker"
        );
        assert_eq!(
            writer.state,
            WriterState::Healthy,
            "successful append keeps writer healthy"
        );
        let summary = verify_journal(&path).unwrap();
        assert_eq!(summary.frame_count, 1, "one complete committed record");
        assert_eq!(
            summary.bytes_scanned as usize,
            bytes.len(),
            "scan covers every written byte"
        );
    }
}
