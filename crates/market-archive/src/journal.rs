use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
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
pub struct JournalWriter {
    file: File,
}

impl JournalWriter {
    pub fn create_new(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        let path = path.as_ref();
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(path)?;
        file.try_lock_exclusive()?;
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(Self { file })
    }

    pub fn open_append(path: impl AsRef<Path>) -> Result<Self, ArchiveError> {
        let path = path.as_ref();
        let file = OpenOptions::new().append(true).open(path)?;
        file.try_lock_exclusive()?;
        // Never append behind a torn or corrupted tail. Validation is streamed
        // while the exclusive lock is held, so reopening a large journal does
        // not require loading every historical frame into memory.
        let _ = verify_journal(path)?;
        Ok(Self { file })
    }

    pub fn append(&mut self, frame: &RawFrame) -> Result<(), ArchiveError> {
        let line = frame.to_json_line()?;
        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.file.sync_data()?;
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
}
