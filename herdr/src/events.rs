//! Events written by the GhostCTO Claude Code hook (`herdr-ghosts`).
//!
//! The hook appends one JSON object per line to `events.jsonl` in the state
//! directory, only when Claude Code runs inside a herdr pane. Every line has
//! `v`, `pane`, `session`, `agent` (a subagent id, empty for the main agent)
//! and `ts` (Unix seconds), plus one of:
//!
//! ```json
//! {"kind":"skill","skill":"ghostcto:committer"}
//! {"kind":"tool","tool":"Bash","detail":"git commit, git push"}
//! {"kind":"stop","transcript":"/path/to/session.jsonl"}
//! ```
//!
//! A line without `kind` is a skill event from the first hook version.
//! The hook may rotate the file to `events.jsonl.1`; the reader starts over
//! when the file shrinks or is replaced.

use serde::Deserialize;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Event {
    pub v: u32,
    pub pane: String,
    #[serde(default)]
    pub session: String,
    #[serde(default)]
    pub agent: String,
    #[serde(default)]
    pub ts: u64,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub skill: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub transcript: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Kind {
    Skill,
    Tool,
    Stop,
}

impl Event {
    pub fn kind(&self) -> Option<Kind> {
        match self.kind.as_str() {
            "" | "skill" if !self.skill.is_empty() => Some(Kind::Skill),
            "tool" if !self.tool.is_empty() => Some(Kind::Tool),
            "stop" => Some(Kind::Stop),
            _ => None,
        }
    }
}

/// Parses one line; anything malformed, of an unknown kind, or from another
/// format version is skipped rather than guessed at.
pub fn parse_line(line: &str) -> Option<Event> {
    let event: Event = serde_json::from_str(line.trim()).ok()?;
    (event.v == 1 && event.kind().is_some()).then_some(event)
}

/// `$XDG_STATE_HOME/ghostcto/herdr`, or `~/.local/state/ghostcto/herdr`.
pub fn state_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_STATE_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    Some(base.join("ghostcto/herdr"))
}

/// Every event already in the file (and its rotated copy, oldest first).
pub fn read_all(path: &Path) -> Vec<Event> {
    let mut rotated = path.as_os_str().to_owned();
    rotated.push(".1");
    [PathBuf::from(rotated), path.to_path_buf()]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .flat_map(|text| text.lines().filter_map(parse_line).collect::<Vec<_>>())
        .collect()
}

/// Follows the end of the events file, returning only lines added since the
/// last read and holding back a line the hook has not finished writing.
pub struct Tail {
    path: PathBuf,
    offset: u64,
    identity: Option<(u64, u64)>,
    partial: String,
}

impl Tail {
    /// Starts at the current end of the file: events from before the pane
    /// opened belong to turns it did not see, so they are not replayed.
    pub fn at_end(path: &Path) -> Self {
        let meta = std::fs::metadata(path).ok();
        Tail {
            path: path.to_path_buf(),
            offset: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            identity: meta.as_ref().map(identity),
            partial: String::new(),
        }
    }

    pub fn read_new(&mut self) -> Vec<Event> {
        let Ok(mut file) = File::open(&self.path) else {
            self.offset = 0;
            self.identity = None;
            return Vec::new();
        };
        let Ok(meta) = file.metadata() else {
            return Vec::new();
        };
        let id = identity(&meta);
        if self.identity != Some(id) || meta.len() < self.offset {
            self.offset = 0;
            self.partial.clear();
        }
        self.identity = Some(id);
        if file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut chunk = String::new();
        let Ok(read) = file.read_to_string(&mut chunk) else {
            return Vec::new();
        };
        self.offset += read as u64;
        self.partial.push_str(&chunk);
        let Some(last_newline) = self.partial.rfind('\n') else {
            return Vec::new();
        };
        let complete: String = self.partial.drain(..=last_newline).collect();
        complete.lines().filter_map(parse_line).collect()
    }
}

#[cfg(unix)]
fn identity(meta: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.dev(), meta.ino())
}

#[cfg(not(unix))]
fn identity(_meta: &std::fs::Metadata) -> (u64, u64) {
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    #[test]
    fn parses_v1_lines_only() {
        let ok = parse_line(r#"{"v":1,"pane":"w9:p1","session":"s","skill":"ghostcto:committer"}"#)
            .unwrap();
        assert_eq!(ok.pane, "w9:p1");
        assert_eq!(ok.skill, "ghostcto:committer");
        assert!(parse_line(r#"{"v":2,"pane":"w9:p1","skill":"x"}"#).is_none());
        assert!(parse_line("not json").is_none());
        assert!(parse_line(r#"{"v":1,"pane":"w9:p1"}"#).is_none());
        let tool = parse_line(r#"{"v":1,"pane":"p","session":"s","agent":"a1","ts":5,"kind":"tool","tool":"Edit","detail":"/x"}"#).unwrap();
        assert_eq!(
            (tool.kind(), tool.agent.as_str(), tool.ts),
            (Some(Kind::Tool), "a1", 5)
        );
        assert_eq!(
            parse_line(r#"{"v":1,"pane":"p","kind":"stop"}"#)
                .unwrap()
                .kind(),
            Some(Kind::Stop)
        );
        assert!(parse_line(r#"{"v":1,"pane":"p","kind":"mystery"}"#).is_none());
    }

    fn append(path: &Path, text: &str) {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    fn temp_file(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ghostcto-herdr-events-{name}-{}.jsonl",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        path
    }

    const LINE: &str =
        "{\"v\":1,\"pane\":\"w1:p1\",\"session\":\"s\",\"skill\":\"ghostcto:start\"}\n";

    #[test]
    fn skips_history_and_reads_new_lines() {
        let path = temp_file("history");
        append(&path, LINE);
        let mut tail = Tail::at_end(&path);
        assert!(tail.read_new().is_empty());
        append(&path, LINE);
        assert_eq!(tail.read_new().len(), 1);
        assert!(tail.read_new().is_empty());
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn holds_back_a_half_written_line() {
        let path = temp_file("partial");
        let mut tail = Tail::at_end(&path);
        let (head, rest) = LINE.split_at(20);
        append(&path, head);
        assert!(tail.read_new().is_empty());
        append(&path, rest);
        assert_eq!(tail.read_new().len(), 1);
        fs::remove_file(&path).unwrap();
    }

    #[test]
    fn starts_over_after_rotation() {
        let path = temp_file("rotate");
        append(&path, LINE);
        append(&path, LINE);
        let mut tail = Tail::at_end(&path);
        fs::rename(&path, path.with_extension("jsonl.1")).unwrap();
        append(&path, LINE);
        assert_eq!(tail.read_new().len(), 1);
        fs::remove_file(&path).unwrap();
        fs::remove_file(path.with_extension("jsonl.1")).unwrap();
    }

    #[test]
    fn read_all_includes_the_rotated_file_first() {
        let path = temp_file("all");
        let mut old = path.as_os_str().to_owned();
        old.push(".1");
        let old = PathBuf::from(old);
        append(&old, &LINE.replace("start", "committer"));
        append(&path, LINE);
        let skills: Vec<_> = read_all(&path).into_iter().map(|e| e.skill).collect();
        assert_eq!(skills, vec!["ghostcto:committer", "ghostcto:start"]);
        fs::remove_file(&path).unwrap();
        fs::remove_file(&old).unwrap();
    }

    #[test]
    fn missing_file_reads_nothing_then_catches_up() {
        let path = temp_file("missing");
        let mut tail = Tail::at_end(&path);
        assert!(tail.read_new().is_empty());
        append(&path, LINE);
        assert_eq!(tail.read_new().len(), 1);
        fs::remove_file(&path).unwrap();
    }
}
