//! Saved block transcripts (opt-in `save_transcripts`).
//!
//! Each pane's finished blocks — command, exit code, duration, directory and
//! output text — are written to `~/.local/share/rift/transcripts/<id>.json`
//! so a relaunch can show them again. They are replayed into the new pane's
//! terminal as ordinary OSC 133 blocks, marked `restored`, above the fresh
//! shell: the output comes back, the processes do not.
//!
//! Bounded: the newest [`MAX_BLOCKS`] blocks and [`MAX_BYTES`] of output per
//! pane. Terminal output can contain secrets; nothing is redacted, which is
//! why the feature is off by default and "Clear saved block transcripts"
//! deletes every file.
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

pub const VERSION: u32 = 1;
pub const MAX_BLOCKS: usize = 200;
pub const MAX_BYTES: usize = 1 << 20;
pub const MAX_FIELD_BYTES: usize = 16 << 10;
/// Worst-case JSON escaping plus per-block metadata.
pub const MAX_FILE_BYTES: usize = MAX_BYTES * 6 + MAX_BLOCKS * 256;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub version: u32,
    /// Seconds since the epoch when it was written.
    pub saved_at: u64,
    pub blocks: Vec<SavedBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedBlock {
    pub command: String,
    pub exit: Option<i32>,
    pub duration_ms: u64,
    pub cwd: Option<String>,
    pub output: String,
}

impl Transcript {
    /// Keep the newest blocks within the block and byte bounds; a single
    /// block over the byte budget keeps its tail.
    pub fn bounded(mut blocks: Vec<SavedBlock>, saved_at: u64) -> Transcript {
        if blocks.len() > MAX_BLOCKS {
            blocks.drain(..blocks.len() - MAX_BLOCKS);
        }
        let mut total = 0usize;
        let mut keep = blocks.len();
        for (i, b) in blocks.iter_mut().enumerate().rev() {
            truncate_field(&mut b.command);
            if let Some(cwd) = &mut b.cwd {
                truncate_field(cwd);
            }
            let fields = b.command.len() + b.cwd.as_ref().map_or(0, String::len);
            let room = MAX_BYTES.saturating_sub(total);
            if room < fields || room == 0 {
                keep = blocks.len() - i - 1;
                break;
            }
            let room = room - fields;
            if b.output.len() > room {
                let mut cut = b.output.len() - room;
                while !b.output.is_char_boundary(cut) {
                    cut += 1;
                }
                b.output = b.output[cut..].to_string();
            }
            total += b.output.len() + fields;
        }
        let skip = blocks.len() - keep;
        Transcript {
            version: VERSION,
            saved_at,
            blocks: blocks.split_off(skip),
        }
    }

    /// Terminal bytes that recreate the blocks: for each, OSC 133 A/B, the
    /// command, C, the output and D with the exit code. Output control
    /// sequences are stripped so replay cannot re-trigger anything (titles,
    /// clipboard writes, mode changes) — only text comes back.
    pub fn replay_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for b in &self.blocks {
            out.extend_from_slice(b"\x1b]133;A\x07");
            out.extend_from_slice(b"\x1b]133;B\x07");
            out.extend_from_slice(sanitize(&b.command.replace('\n', " ")).as_bytes());
            out.extend_from_slice(b"\r\n\x1b]133;C\x07");
            for line in b.output.lines() {
                out.extend_from_slice(sanitize(line).as_bytes());
                out.extend_from_slice(b"\r\n");
            }
            let exit = b.exit.map(|e| e.to_string()).unwrap_or_default();
            out.extend_from_slice(format!("\x1b]133;D;{exit}\x07").as_bytes());
        }
        out
    }

    /// (duration, cwd, command) per block, for `BlockStore::mark_restored`.
    /// The command keeps its newlines and tabs but no other control
    /// characters: it can be reinput and run again.
    pub fn meta(&self) -> Vec<crate::Restored> {
        self.blocks
            .iter()
            .map(|b| {
                let duration = Duration::from_millis(b.duration_ms);
                let command = b
                    .command
                    .chars()
                    .filter(|&c| c == '\n' || c == '\t' || !c.is_control())
                    .collect();
                (duration, b.cwd.clone(), command)
            })
            .collect()
    }

    fn valid(&self) -> bool {
        self.version == VERSION
            && self.blocks.len() <= MAX_BLOCKS
            && self.blocks.iter().all(|b| {
                b.command.len() <= MAX_FIELD_BYTES
                    && b.cwd.as_ref().is_none_or(|c| c.len() <= MAX_FIELD_BYTES)
            })
            && self
                .blocks
                .iter()
                .try_fold(0usize, |total, b| {
                    total
                        .checked_add(b.command.len())?
                        .checked_add(b.output.len())?
                        .checked_add(b.cwd.as_ref().map_or(0, String::len))
                })
                .is_some_and(|total| total <= MAX_BYTES)
    }
}

fn truncate_field(s: &mut String) {
    let mut end = s.len().min(MAX_FIELD_BYTES);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// Text only: drop ESC and other C0 controls except tab.
fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| *c == '\t' || !c.is_control())
        .collect()
}

pub fn dir() -> Option<PathBuf> {
    crate::shell_integration::data_dir().map(|d| d.join("transcripts"))
}

fn path(id: &str) -> Option<PathBuf> {
    // ids are generated by the app; refuse anything path-like
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    dir().map(|d| d.join(format!("{id}.json")))
}

/// A fresh transcript id (process id + time + counter).
pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// Atomic, private (0600) write.
pub fn save(id: &str, t: &Transcript) {
    if !t.valid() {
        return;
    }
    let (Some(path), Some(dir)) = (path(id), dir()) else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(body) = serde_json::to_vec(t) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let ok = opts
        .open(&tmp)
        .and_then(|mut f| std::io::Write::write_all(&mut f, &body))
        .is_ok();
    if ok {
        let _ = std::fs::rename(&tmp, &path);
    }
}

pub fn load(id: &str) -> Option<Transcript> {
    let path = path(id)?;
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES as u64 {
        return None;
    }
    load_reader(std::fs::File::open(path).ok()?)
}

fn load_reader(reader: impl std::io::Read) -> Option<Transcript> {
    let mut body = Vec::new();
    reader
        .take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .ok()?;
    if body.len() > MAX_FILE_BYTES {
        return None;
    }
    let t: Transcript = serde_json::from_slice(&body).ok()?;
    t.valid().then_some(t)
}

/// Delete every saved transcript except `keep` (ids still referenced by the
/// current session). Returns how many files were removed.
pub fn remove_except(keep: &[String]) -> usize {
    let Some(dir) = dir() else { return 0 };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut n = 0;
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let id = name.trim_end_matches(".json").trim_end_matches(".json.tmp");
        if !keep.iter().any(|k| k == id) && std::fs::remove_file(e.path()).is_ok() {
            n += 1;
        }
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(cmd: &str, out: &str) -> SavedBlock {
        SavedBlock {
            command: cmd.into(),
            exit: Some(0),
            duration_ms: 12,
            cwd: Some("/tmp".into()),
            output: out.into(),
        }
    }

    #[test]
    fn bounds_keep_the_newest() {
        let many: Vec<SavedBlock> = (0..MAX_BLOCKS + 5)
            .map(|i| block(&format!("c{i}"), "x"))
            .collect();
        let t = Transcript::bounded(many, 1);
        assert_eq!(t.blocks.len(), MAX_BLOCKS);
        assert_eq!(
            t.blocks.last().unwrap().command,
            format!("c{}", MAX_BLOCKS + 4)
        );

        let big = "é".repeat(MAX_BYTES); // 2 bytes per char
        let t = Transcript::bounded(vec![block("old", "gone"), block("big", &big)], 1);
        assert_eq!(t.blocks.len(), 1, "the older block no longer fits");
        assert!(t.blocks[0].output.len() <= MAX_BYTES);
        assert!(t.blocks[0].output.chars().all(|c| c == 'é'));
    }

    #[test]
    fn replay_is_text_only_osc133() {
        let t = Transcript::bounded(
            vec![SavedBlock {
                exit: Some(2),
                ..block("ls\nx", "a\x1b]52;c;ZXZpbA==\x07b\n\x1b[31mred")
            }],
            1,
        );
        let bytes = String::from_utf8(t.replay_bytes()).unwrap();
        assert_eq!(
            bytes,
            "\x1b]133;A\x07\x1b]133;B\x07ls x\r\n\x1b]133;C\x07a]52;c;ZXZpbA==b\r\n[31mred\r\n\x1b]133;D;2\x07"
        );
    }

    #[test]
    fn ids_cannot_escape_the_directory() {
        assert!(path("../../etc/passwd").is_none());
        assert!(path("").is_none());
        assert!(path("123-456-0").is_some() || dir().is_none());
        assert_ne!(new_id(), new_id());
    }

    #[test]
    fn roundtrip_json() {
        let t = Transcript::bounded(vec![block("echo hi", "hi\n")], 42);
        let s = serde_json::to_string(&t).unwrap();
        let back: Transcript = serde_json::from_str(&s).unwrap();
        assert_eq!(back, t);
        assert_eq!(back.meta()[0].0, Duration::from_millis(12));
        // the exact command survives, minus control characters
        let t = Transcript::bounded(vec![block("printf 'a\n\tb'\x1b]0;x\x07 e\u{301}", "")], 42);
        assert_eq!(t.meta()[0].2, "printf 'a\n\tb']0;x e\u{301}");
    }

    #[test]
    fn restored_data_is_bounded_before_replay() {
        let read =
            |t: &Transcript| load_reader(std::io::Cursor::new(serde_json::to_vec(t).unwrap()));
        let valid = Transcript::bounded(vec![block("hi", "text")], 1);
        assert_eq!(read(&valid), Some(valid.clone()));
        let mut bad = valid.clone();
        bad.version += 1;
        assert!(read(&bad).is_none());
        bad = valid.clone();
        bad.blocks = vec![block("", ""); MAX_BLOCKS + 1];
        assert!(read(&bad).is_none());
        for field in ["command", "cwd", "output"] {
            let mut bad = valid.clone();
            match field {
                "command" => bad.blocks[0].command = "x".repeat(MAX_FIELD_BYTES + 1),
                "cwd" => bad.blocks[0].cwd = Some("x".repeat(MAX_FIELD_BYTES + 1)),
                _ => bad.blocks[0].output = "x".repeat(MAX_BYTES + 1),
            }
            assert!(read(&bad).is_none());
            let bounded = Transcript::bounded(bad.blocks, 1);
            assert_eq!(read(&bounded), Some(bounded));
        }
        assert!(load_reader(std::io::repeat(b' ')).is_none());
        assert!(load_reader(&b"{broken"[..]).is_none());
        let escaped = Transcript::bounded(vec![block("\0", &"\0".repeat(MAX_BYTES))], 1);
        assert_eq!(read(&escaped), Some(escaped));
    }
}
