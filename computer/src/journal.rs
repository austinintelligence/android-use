//! Bounded at-most-once operation journal shared by every Android Use process
//! on this computer.
//!
//! Each mutation is recorded as `pending` (durably) before it is dispatched and
//! as `done` with its receipt afterwards. A replayed operation id returns the
//! stored receipt, or `unknown` while its outcome is still pending, so a
//! mutation is never silently repeated.
//!
//! A pending record that outlives the process that wrote it means the device
//! may have acted without the host learning the outcome. The journal then
//! fences new mutations: they return `unknown` until the agent reads the same
//! surface, which calls [`Journal::reconcile`] and resolves them as `unknown`.
//! Every access takes an exclusive lock on a sidecar file and reloads the
//! journal first, so the CLI and the MCP server can share one journal.
use crate::{
    api::{Code, Error, Receipt, Result},
    artifact::valid,
    device::{atomic, private_file, recover, Paths},
    engine::session_id,
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, OpenOptions},
    io::{ErrorKind, Read, Seek, Write},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_FILE: u64 = 2 * 1024 * 1024;
const COMPACT_AT: u64 = 1024 * 1024;
const MAX_RECORD: usize = 2048;
const MAX_ENTRIES: usize = 1024;
const REUSED: &str = "operation id was already used for a different plan";
pub(crate) const ANDROID: &str = "a";
pub(crate) const BROWSER: &str = "b";

#[derive(Clone)]
enum State {
    Pending { digest: Box<str>, generation: u64, owner: Box<str>, surface: Box<str> },
    Done { digest: Box<str>, receipt: Receipt },
}

#[derive(Default)]
pub(crate) struct Journal {
    path: PathBuf,
    lock_path: PathBuf,
    states: HashMap<Box<str>, State>,
    order: VecDeque<Box<str>>,
    owner: Box<str>,
    held: Option<fs::File>,
    /// (lock-file write counter, journal length and mtime) as of this process's last load or write.
    seen: Option<(u64, u64, Option<SystemTime>)>,
    wrote: bool,
    #[cfg(test)]
    loads: u32,
}

impl Drop for Journal {
    fn drop(&mut self) {
        let _ = self.held.take().map(|_| fs::remove_file(self.path.with_extension(format!("owner-{}", self.owner))));
    }
}

/// A client resent a request id whose earlier attempt may have acted. Say so
/// in agent terms instead of reporting a malformed request.
pub(crate) fn explain_reuse(error: Error) -> Error {
    if error.code == Code::Args && error.message == REUSED {
        Error::plain(Code::Unknown, "This request was already sent once and may have run. Read the screen before doing anything else.")
    } else {
        error
    }
}

pub(crate) fn unknown(id: &str, generation: u64) -> Receipt {
    Receipt { id: id.into(), ok: 0, g: generation, m: 0, at: None, e: Some("unknown".into()), partial: None, next: Some("observe".into()), artifact: None }
}

impl Journal {
    pub(crate) fn open(paths: &Paths) -> Result<Self> {
        let mut journal = Self::default();
        (journal.path, journal.lock_path, journal.owner) = (paths.journal.clone(), paths.journal.with_extension("lock"), session_id().into());
        journal.locked(|journal| journal.reclaim_owners())?;
        Ok(journal)
    }

    /// Records a pending operation, or returns the outcome of an earlier
    /// attempt with the same id and digest. While an earlier process left a
    /// pending record behind, new operations return `unknown` without being
    /// journaled or dispatched.
    pub(crate) fn begin(&mut self, id: &str, generation: u64, digest: &str, surface: &str) -> Result<Option<Receipt>> {
        self.locked(|journal| {
            if let Some(state) = journal.states.get(id) {
                return match state {
                    State::Pending { digest: d, generation, .. } if d.as_ref() == digest => Ok(Some(unknown(id, *generation))),
                    State::Done { digest: d, receipt } if d.as_ref() == digest => Ok(Some(receipt.clone())),
                    _ => Err(Error::new(Code::Args, REUSED)),
                };
            }
            if journal.states.values().any(|state| matches!(state, State::Pending { .. })) {
                return Ok(Some(unknown(id, generation)));
            }
            if journal.states.len() >= MAX_ENTRIES {
                journal.trim(MAX_ENTRIES - 1)?;
                journal.compact()?;
            }
            // The pending record must reach the disk before dispatch: it is what
            // turns a crash mid-operation into `unknown` instead of a replay.
            if journal.held.is_none() {
                let held = private_file().create(true).truncate(false).write(true).open(journal.path.with_extension(format!("owner-{}", journal.owner)))?;
                held.lock()?;
                journal.held = Some(held);
            }
            let state = State::Pending { digest: digest.into(), generation, owner: journal.owner.clone(), surface: surface.into() };
            journal.append(record(id, &state), true)?;
            Ok(None)
        })
    }

    /// Records the receipt. Not forced to disk: if it is lost, the record stays
    /// pending and the next process fences mutations until the agent reads.
    pub(crate) fn finish(&mut self, digest: &str, receipt: &Receipt) -> Result<()> {
        self.locked(|journal| journal.append(json!([receipt.id, "d", digest, receipt]), false))
    }

    /// Resolves as `unknown` the outcomes no running operation can still settle,
    /// lifting the fence. Call only after the agent has read `surface`.
    pub(crate) fn reconcile(&mut self, surface: &str) -> Result<usize> {
        self.locked(|journal| {
            let settled: Vec<Value> = journal.states.iter().filter_map(|(id, state)| journal.settle(id, state, surface)).collect();
            settled.iter().for_each(|value| journal.apply(value));
            if !settled.is_empty() {
                journal.compact()?;
            }
            Ok(settled.len())
        })
    }

    /// Runs `f` under the journal lock with the file current, so every decision
    /// is made on what other processes have already written. The lock file holds
    /// a counter every writer bumps; the journal is re-parsed only when that
    /// counter or the file length differs from what this process last saw.
    fn locked<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let mut lock = private_file().create(true).truncate(false).read(true).write(true).open(&self.lock_path)?;
        lock.lock()?;
        let mut raw = [0; 8];
        let count = lock.read_exact(&mut raw).map_or(0, |()| u64::from_le_bytes(raw));
        let path = self.path.clone();
        let length = || fs::metadata(&path).map_or((0, None), |m| (m.len(), m.modified().ok()));
        let before = length();
        let result = if self.seen == Some((count, before.0, before.1)) { Ok(()) } else { self.reload() }.and_then(|()| f(self));
        let after = length();
        self.seen = result.is_ok().then_some((count, after.0, after.1));
        if after != before || std::mem::take(&mut self.wrote) {
            let next = count.wrapping_add(1);
            if lock.rewind().and_then(|()| lock.write_all(&next.to_le_bytes())).is_ok() {
                self.seen = self.seen.map(|(_, length, time)| (next, length, time));
            } else {
                self.seen = None;
            }
        }
        result
    }

    /// The `unknown` outcome for a pending record that a read of `read` settles
    /// (its surface, or "" for any). This journal has nothing in flight while it
    /// reconciles; any other holds an OS lock on its owner file from its first
    /// mutation until it drops, and the OS releases that lock if it dies.
    fn settle(&self, id: &str, state: &State, read: &str) -> Option<Value> {
        let State::Pending { digest, generation, owner, surface } = state else { return None };
        let gone = || !owner.bytes().all(|b| b.is_ascii_hexdigit()) || fs::File::open(self.path.with_extension(format!("owner-{owner}"))).map_or(true, |f| f.try_lock().is_ok());
        let settled = ["", read].contains(&&**surface) && (*owner == self.owner || gone());
        settled.then(|| record(id, &State::Done { digest: digest.clone(), receipt: unknown(id, *generation) }))
    }

    fn reload(&mut self) -> Result<()> {
        #[cfg(test)]
        {
            self.loads += 1;
        }
        self.states.clear();
        self.order.clear();
        recover(&self.path)?;
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error.into()),
        };
        if bytes.len() as u64 > MAX_FILE {
            return self.set_aside(&bytes);
        }
        // A record without its newline was torn by a crash (appends hold the
        // lock, so no writer is mid-record). Drop it before anything follows it.
        let complete = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |index| index + 1);
        if complete < bytes.len() {
            OpenOptions::new().write(true).open(&self.path)?.set_len(complete as u64)?;
        }
        for line in String::from_utf8_lossy(&bytes[..complete]).lines().filter(|line| line.len() <= MAX_RECORD) {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                self.apply(&value);
            }
        }
        if self.trim(MAX_ENTRIES)? {
            self.compact()?;
        }
        Ok(())
    }

    /// Removes owner files whose process has exited. A live owner holds its file's OS lock, so an
    /// unlocked file is stale. Owners are created under the journal lock, which this runs under too.
    fn reclaim_owners(&self) -> Result<()> {
        let (Some(dir), Some(stem)) = (self.path.parent(), self.path.file_stem().and_then(|stem| stem.to_str())) else { return Ok(()) };
        let prefix = format!("{stem}.owner-");
        for entry in fs::read_dir(dir)?.flatten() {
            if !entry.file_name().to_str().is_some_and(|name| name.starts_with(&prefix)) {
                continue;
            }
            // Owners start only under the journal lock, so a file that takes its lock has no live owner.
            if let Ok(file) = fs::File::open(entry.path()) {
                if file.try_lock().is_ok() {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }

    /// Saves an oversized journal aside and starts a fresh one. The fresh journal keeps the
    /// pending records, which still fence new mutations, and the newest completed receipts.
    /// The old file is copied before it is replaced, so a crash leaves a journal either way.
    fn set_aside(&mut self, bytes: &[u8]) -> Result<()> {
        let mut kept = Self::default();
        kept.path = self.path.clone();
        for line in String::from_utf8_lossy(bytes).lines().filter(|line| line.len() <= MAX_RECORD) {
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                kept.apply(&value);
            }
        }
        kept.trim(MAX_ENTRIES)?;
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let aside = self.path.with_extension(format!("oversized-{nanos}"));
        private_file().write(true).create(true).truncate(true).open(&aside)?.write_all(bytes)?;
        eprintln!("warning: operation journal exceeded 2 MiB; saved it to {} and started a new one", aside.display());
        kept.compact()?;
        self.states = std::mem::take(&mut kept.states);
        self.order = std::mem::take(&mut kept.order);
        Ok(())
    }

    fn apply(&mut self, v: &Value) {
        let Some(a) = v.as_array() else { return };
        let (Some(id), Some(kind), Some(digest)) = (a.first().and_then(Value::as_str), a.get(1).and_then(Value::as_str), a.get(2).and_then(Value::as_str)) else { return };
        if !valid(id) || digest.len() != 64 {
            return;
        }
        let state = match kind {
            "p" => a.get(3).and_then(Value::as_u64).map(|generation| {
                let text = |i: usize| a.get(i).and_then(Value::as_str).unwrap_or_default().into();
                State::Pending { digest: digest.into(), generation, owner: text(4), surface: text(5) }
            }),
            "d" => a.get(3).cloned().and_then(|v| serde_json::from_value::<Receipt>(v).ok()).map(|receipt| State::Done { digest: digest.into(), receipt }),
            _ => None,
        };
        // A rejected record leaves both the state and its place in `order` alone.
        let Some(state) = state else { return };
        if self.states.contains_key(id) {
            if let Some(pos) = self.order.iter().rposition(|v| v.as_ref() == id) {
                self.order.remove(pos);
            }
        }
        self.order.push_back(id.into());
        self.states.insert(id.into(), state);
    }

    /// Appends one record in a single write. If the write fails, the partial
    /// record is rolled back so later records are never glued onto it.
    fn append(&mut self, v: Value, durable: bool) -> Result<()> {
        let mut line = serde_json::to_vec(&v).map_err(|e| Error::new(Code::Io, e.to_string()))?;
        if line.len() > MAX_RECORD {
            return Err(Error::new(Code::Bounds, "journal record exceeds limit"));
        }
        line.push(b'\n');
        self.wrote = true;
        let mut file = private_file().create(true).append(true).open(&self.path)?;
        let before = file.metadata()?.len();
        let written = file.write_all(&line).and_then(|()| if durable { file.sync_all() } else { Ok(()) });
        if let Err(error) = written {
            let _ = file.set_len(before);
            return Err(error.into());
        }
        self.apply(&v);
        let size = file.metadata()?.len();
        if self.trim(MAX_ENTRIES)? || size > COMPACT_AT {
            drop(file);
            self.compact()?;
        }
        Ok(())
    }

    /// Evicts the oldest completed operations. Pending operations are never
    /// evicted: forgetting one could let an uncertain mutation be replayed.
    fn trim(&mut self, max: usize) -> Result<bool> {
        let mut changed = false;
        while self.states.len() > max {
            let Some(pos) = self.order.iter().position(|id| matches!(self.states.get(id.as_ref()), Some(State::Done { .. }))) else {
                return Err(Error::new(Code::Bounds, "operation journal is full of unresolved operations"));
            };
            if let Some(id) = self.order.remove(pos) {
                self.states.remove(id.as_ref());
                changed = true;
            }
        }
        Ok(changed)
    }

    fn compact(&mut self) -> Result<()> {
        self.wrote = true;
        let mut bytes = Vec::new();
        for id in &self.order {
            let Some(state) = self.states.get(id.as_ref()) else { continue };
            bytes.extend_from_slice(&serde_json::to_vec(&record(id, state)).map_err(|e| Error::new(Code::Io, e.to_string()))?);
            bytes.push(b'\n');
        }
        atomic(&self.path, &bytes)
    }
}

fn record(id: &str, state: &State) -> Value {
    match state {
        State::Pending { digest, generation, owner, surface } => json!([id, "p", digest, generation, owner, surface]),
        State::Done { digest, receipt } => json!([id, "d", digest, receipt]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn receipt(id: &str) -> Receipt {
        Receipt { id: id.into(), ok: 1, g: 42, m: 1, at: None, e: None, partial: None, next: None, artifact: None }
    }

    fn open(dir: &tempfile::TempDir) -> Journal {
        Journal::open(&Paths::at(dir.path().to_path_buf()).unwrap()).unwrap()
    }

    #[test]
    fn journal_unknown_and_dedupe() {
        let d = tempfile::tempdir().unwrap();
        let mut j = open(&d);
        assert!(j.begin("8", 41, A, ANDROID).unwrap().is_none());
        assert_eq!(j.begin("8", 41, A, ANDROID).unwrap().unwrap().e.as_deref(), Some("unknown"));
        assert_eq!(j.begin("8", 41, B, ANDROID).unwrap_err().code, Code::Args);
    }

    #[test]
    fn a_crashed_pending_record_fences_new_mutations_until_reconciled() {
        let d = tempfile::tempdir().unwrap();
        {
            let mut j = open(&d);
            assert!(j.begin("finished", 7, B, ANDROID).unwrap().is_none());
            j.finish(B, &receipt("finished")).unwrap();
            // Left pending: the process "crashes" before recording an outcome.
            assert!(j.begin("crashed", 7, A, ANDROID).unwrap().is_none());
        }
        let mut j = open(&d);
        assert_eq!(j.begin("crashed", 7, A, ANDROID).unwrap().unwrap().e.as_deref(), Some("unknown"), "same id is unknown");
        assert_eq!(j.begin("finished", 7, B, ANDROID).unwrap().unwrap().ok, 1, "completed ids replay their receipt");
        let fresh = j.begin("fresh", 9, A, ANDROID).unwrap().unwrap();
        assert_eq!(fresh.e.as_deref(), Some("unknown"), "new mutations are fenced");
        assert_eq!(j.reconcile(ANDROID).unwrap(), 1);
        assert!(j.begin("fresh", 9, A, ANDROID).unwrap().is_none(), "reconciling lifts the fence");
        let mut reopened = open(&d);
        assert_eq!(reopened.begin("crashed", 7, A, ANDROID).unwrap().unwrap().e.as_deref(), Some("unknown"), "reconciled outcome persists");
        assert!(reopened.begin("fresh", 9, A, ANDROID).unwrap().is_some(), "a reopened journal stays consistent");
    }

    #[test]
    fn a_torn_record_is_dropped_and_never_glues_onto_the_next_one() {
        let d = tempfile::tempdir().unwrap();
        let path = Paths::at(d.path().to_path_buf()).unwrap().journal;
        {
            let mut j = open(&d);
            j.begin("first", 1, A, ANDROID).unwrap();
            j.finish(A, &receipt("first")).unwrap();
        }
        let intact = fs::read(&path).unwrap();
        let mut torn = intact.clone();
        torn.extend_from_slice(br#"["torn","p","#);
        fs::write(&path, &torn).unwrap();
        let mut j = open(&d);
        assert_eq!(fs::read(&path).unwrap(), intact, "the torn tail is removed on open");
        assert!(j.begin("second", 2, B, ANDROID).unwrap().is_none());
        j.finish(B, &receipt("second")).unwrap();
        let reopened = open(&d);
        assert!(matches!(reopened.states.get("second"), Some(State::Done { .. })), "the record after a repair is not lost");
    }

    #[test]
    fn two_processes_share_one_journal_under_the_lock() {
        let d = tempfile::tempdir().unwrap();
        let mut server = open(&d);
        let mut cli = open(&d);
        assert!(cli.begin("pay", 3, A, ANDROID).unwrap().is_none());
        assert_eq!(server.begin("other", 3, B, ANDROID).unwrap().unwrap().e.as_deref(), Some("unknown"), "the server sees the CLI's pending record");
        assert_eq!(server.reconcile(ANDROID).unwrap(), 0, "a live sibling's operation may still be running");
        drop(cli);
        assert_eq!(server.reconcile(BROWSER).unwrap(), 0, "a Chrome read does not settle an Android action");
        assert_eq!(server.reconcile(ANDROID).unwrap(), 1, "an exited owner's record is settled by a read of its surface");
        assert!(server.begin("after", 4, B, ANDROID).unwrap().is_none());
    }

    #[test]
    fn legacy_pending_records_settle_on_any_read_and_a_moved_aside_journal_is_restored() {
        let d = tempfile::tempdir().unwrap();
        let path = Paths::at(d.path().to_path_buf()).unwrap().journal;
        fs::write(path.with_extension("bak-1-2"), format!("[\"old\",\"p\",\"{A}\",1]\n")).unwrap();
        let mut j = open(&d);
        assert!(j.begin("new", 2, B, BROWSER).unwrap().is_some(), "the backup's pending record still fences");
        assert_eq!(j.reconcile(BROWSER).unwrap(), 1);
    }

    #[test]
    fn pending_records_are_never_evicted() {
        let d = tempfile::tempdir().unwrap();
        let mut j = open(&d);
        for i in 0..(MAX_ENTRIES + 4) {
            j.apply(&json!([format!("p{i}"), "p", A, 1]));
        }
        assert!(j.trim(MAX_ENTRIES).is_err(), "a journal of only pending records refuses to evict any");
        assert!(j.states.contains_key("p0"));
    }

    #[test]
    fn completed_operations_are_evicted_oldest_first() {
        let d = tempfile::tempdir().unwrap();
        let mut j = open(&d);
        j.apply(&json!(["keep", "p", A, 1]));
        for i in 0..(MAX_ENTRIES + 8) {
            let id = format!("done{i}");
            j.apply(&json!([id, "d", B, receipt(&id)]));
        }
        assert!(j.trim(MAX_ENTRIES).unwrap());
        assert_eq!(j.states.len(), MAX_ENTRIES);
        assert!(j.states.contains_key("keep"), "pending survives eviction");
        assert!(!j.states.contains_key("done0"));
    }

    fn aside_files(dir: &tempfile::TempDir) -> Vec<String> {
        fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()?.file_name().into_string().ok()).filter(|n| n.contains(".oversized-")).collect()
    }

    #[test]
    fn an_oversized_journal_is_saved_aside_once_and_keeps_its_pending_records() {
        let d = tempfile::tempdir().unwrap();
        let path = Paths::at(d.path().to_path_buf()).unwrap().journal;
        let mut bytes = format!("{}\n", json!(["crashed", "p", A, 1, "gone", ""])).into_bytes();
        bytes.extend(vec![b'#'; MAX_FILE as usize + 1]);
        bytes.push(b'\n');
        fs::write(&path, &bytes).unwrap();
        let mut j = open(&d);
        assert_eq!(aside_files(&d).len(), 1, "the oversized file was saved aside");
        assert_eq!(fs::read(&path).unwrap().len(), format!("{}\n", json!(["crashed", "p", A, 1, "gone", ""])).len());
        assert_eq!(j.begin("fresh", 2, B, ANDROID).unwrap().unwrap().e.as_deref(), Some("unknown"), "the pending record still fences");
        drop(j);
        let _ = open(&d);
        assert_eq!(aside_files(&d).len(), 1, "a fresh journal is not saved aside again");
    }

    #[test]
    fn a_stale_owner_file_is_reclaimed_and_a_live_one_is_kept() {
        let d = tempfile::tempdir().unwrap();
        let paths = Paths::at(d.path().to_path_buf()).unwrap();
        let stale = paths.journal.with_extension("owner-stale");
        fs::write(&stale, b"").unwrap();
        let live = paths.journal.with_extension("owner-live");
        let held = fs::File::create(&live).unwrap();
        held.lock().unwrap();
        let _ = open(&d);
        assert!(!stale.exists(), "an owner whose process exited is reclaimed");
        assert!(live.exists(), "an owner that still holds its lock is kept");
        drop(held);
    }

    #[test]
    fn many_acts_in_one_process_parse_the_journal_once() {
        let d = tempfile::tempdir().unwrap();
        let mut j = open(&d);
        let start = j.loads;
        for i in 0..50 {
            let id = format!("op{i}");
            assert!(j.begin(&id, 1, A, ANDROID).unwrap().is_none());
            j.finish(A, &receipt(&id)).unwrap();
        }
        assert_eq!(j.loads, start, "own writes never trigger a reload");
    }

    #[test]
    fn another_handle_writing_bumps_the_counter_and_is_seen() {
        let d = tempfile::tempdir().unwrap();
        let mut first = open(&d);
        let mut second = open(&d);
        assert!(first.begin("warm", 1, A, ANDROID).unwrap().is_none());
        first.finish(A, &receipt("warm")).unwrap();
        let counter = || fs::read(first.lock_path.clone()).unwrap();
        let before = counter();
        let loads = first.loads;
        assert!(second.begin("theirs", 1, B, ANDROID).unwrap().is_none());
        assert_ne!(counter(), before, "a writer bumps the lock-file counter");
        assert_eq!(first.begin("theirs", 1, B, ANDROID).unwrap().unwrap().e.as_deref(), Some("unknown"), "the other write is seen");
        assert_eq!(first.loads, loads + 1);
    }

    #[test]
    fn a_same_length_rewrite_is_seen_through_the_counter_and_bad_records_leave_no_ghosts() {
        let d = tempfile::tempdir().unwrap();
        let mut first = open(&d);
        assert!(first.begin("ab", 1, A, ANDROID).unwrap().is_none());
        first.finish(A, &receipt("ab")).unwrap();
        first.apply(&json!(["bad", "p", A, "x"]));
        first.apply(&json!(["bad", "p", A, 1]));
        assert_eq!(first.order.len(), first.states.len(), "a malformed record leaves no ghost in the order queue");
        let path = first.path.clone();
        let before = fs::read(&path).unwrap();
        fs::write(&path, String::from_utf8(before.clone()).unwrap().replace("\"ab\"", "\"cd\"")).unwrap();
        assert_eq!(fs::read(&path).unwrap().len(), before.len());
        let lock = first.lock_path.clone();
        let count = u64::from_le_bytes(fs::read(&lock).unwrap()[..8].try_into().unwrap());
        fs::write(&lock, (count + 1).to_le_bytes()).unwrap();
        assert!(first.begin("cd", 1, A, ANDROID).unwrap().is_some(), "the rewritten journal is reloaded");
    }

    #[test]
    fn a_rejected_record_for_a_pending_id_keeps_its_fence_through_compaction() {
        let d = tempfile::tempdir().unwrap();
        let mut j = open(&d);
        assert!(j.begin("ab", 1, A, ANDROID).unwrap().is_none());
        j.apply(&json!(["ab", "p", A, "x"]));
        j.apply(&json!(["ab", "z", A]));
        assert_eq!(j.order.iter().map(|id| &**id).collect::<Vec<_>>(), ["ab"]);
        j.compact().unwrap();
        drop(j);
        let mut reopened = open(&d);
        assert_eq!(reopened.begin("ab", 1, A, ANDROID).unwrap().unwrap().e.as_deref(), Some("unknown"), "the pending record survives");
    }

    #[test]
    fn reused_ids_explain_themselves_to_the_agent() {
        let explained = explain_reuse(Error::new(Code::Args, REUSED));
        assert!(explained.plain && explained.code == Code::Unknown);
        assert_eq!(explain_reuse(Error::new(Code::Args, "other")).code, Code::Args);
    }
}
