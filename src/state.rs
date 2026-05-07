use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub type SessionId = String;
pub type PaneRef = String;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Working,
    Waiting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TmuxLocation {
    pub pane: PaneRef,
    pub socket: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub session_id: SessionId,
    pub cwd: Option<PathBuf>,
    pub tmux: Option<TmuxLocation>,
    pub status: Status,
    pub last_message: Option<String>,
    pub waiting_since_secs: Option<u64>,
    pub age_secs: u64,
    #[serde(skip, default = "Instant::now")]
    started_at: Instant,
    #[serde(skip)]
    waiting_since: Option<Instant>,
}

impl Session {
    fn new(session_id: SessionId, cwd: Option<PathBuf>, tmux: Option<TmuxLocation>) -> Self {
        Self {
            session_id,
            cwd,
            tmux,
            status: Status::Working,
            last_message: None,
            waiting_since_secs: None,
            age_secs: 0,
            started_at: Instant::now(),
            waiting_since: None,
        }
    }

    pub fn refresh_durations(&mut self) {
        self.age_secs = self.started_at.elapsed().as_secs();
        self.waiting_since_secs = self.waiting_since.map(|t| t.elapsed().as_secs());
    }
}

pub struct State {
    pub sessions: HashMap<SessionId, Session>,
    pub by_pane: HashMap<PaneRef, SessionId>,
    last_activity: Instant,
}

impl Default for State {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            by_pane: HashMap::new(),
            last_activity: Instant::now(),
        }
    }
}

pub type Shared = Arc<RwLock<State>>;

pub fn new_shared() -> Shared {
    Arc::new(RwLock::new(State::default()))
}

impl State {
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn is_idle(&self, threshold: Duration) -> bool {
        self.sessions.is_empty() && self.last_activity.elapsed() >= threshold
    }

    pub fn waiting_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.status == Status::Waiting)
            .count()
    }

    /// Insert (or update) a session and place it in Working state.
    pub fn upsert_working(
        &mut self,
        session_id: SessionId,
        cwd: Option<PathBuf>,
        tmux: Option<TmuxLocation>,
    ) {
        if let Some(loc) = &tmux {
            self.by_pane.insert(loc.pane.clone(), session_id.clone());
        }
        let entry = self
            .sessions
            .entry(session_id.clone())
            .or_insert_with(|| Session::new(session_id, cwd.clone(), tmux.clone()));
        entry.status = Status::Working;
        entry.waiting_since = None;
        if cwd.is_some() {
            entry.cwd = cwd;
        }
        if tmux.is_some() {
            entry.tmux = tmux;
        }
    }

    /// Mark a session as Waiting. Creates the session if missing.
    pub fn mark_waiting(
        &mut self,
        session_id: SessionId,
        cwd: Option<PathBuf>,
        tmux: Option<TmuxLocation>,
        message: Option<String>,
    ) {
        if let Some(loc) = &tmux {
            self.by_pane.insert(loc.pane.clone(), session_id.clone());
        }
        let entry = self
            .sessions
            .entry(session_id.clone())
            .or_insert_with(|| Session::new(session_id, cwd.clone(), tmux.clone()));
        entry.status = Status::Waiting;
        entry.waiting_since = Some(Instant::now());
        entry.last_message = message.or(entry.last_message.take());
        if cwd.is_some() {
            entry.cwd = cwd;
        }
        if tmux.is_some() {
            entry.tmux = tmux;
        }
    }

    /// Drop a session entirely (SessionEnd).
    pub fn drop_session(&mut self, session_id: &str) {
        if let Some(s) = self.sessions.remove(session_id)
            && let Some(loc) = s.tmux
        {
            self.by_pane.remove(&loc.pane);
        }
    }

    /// User visited a pane. Clear Waiting status for the session bound to that pane.
    /// Returns true if a session was cleared.
    pub fn visit_pane(&mut self, pane: &str) -> bool {
        let Some(sid) = self.by_pane.get(pane).cloned() else {
            return false;
        };
        let Some(s) = self.sessions.get_mut(&sid) else {
            return false;
        };
        if s.status == Status::Waiting {
            s.status = Status::Working;
            s.waiting_since = None;
            true
        } else {
            false
        }
    }

    pub fn snapshot(&self) -> Vec<Session> {
        let mut out: Vec<Session> = self.sessions.values().cloned().collect();
        for s in &mut out {
            s.refresh_durations();
        }
        // Waiting first, then by oldest waiting_since
        out.sort_by(|a, b| {
            (b.status == Status::Waiting)
                .cmp(&(a.status == Status::Waiting))
                .then(b.waiting_since_secs.cmp(&a.waiting_since_secs))
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(p: &str) -> Option<TmuxLocation> {
        Some(TmuxLocation {
            pane: p.to_string(),
            socket: None,
        })
    }

    #[test]
    fn upsert_working_creates_session_and_indexes_pane() {
        let mut s = State::default();
        s.upsert_working("S1".into(), Some("/tmp/a".into()), pane("%1"));

        assert_eq!(s.sessions.len(), 1);
        let session = &s.sessions["S1"];
        assert_eq!(session.status, Status::Working);
        assert_eq!(session.cwd.as_deref(), Some(std::path::Path::new("/tmp/a")));
        assert_eq!(s.by_pane["%1"], "S1");
    }

    #[test]
    fn upsert_working_flips_waiting_back_to_working() {
        let mut s = State::default();
        s.mark_waiting("S1".into(), None, pane("%1"), Some("hi".into()));
        assert_eq!(s.sessions["S1"].status, Status::Waiting);
        assert!(s.sessions["S1"].waiting_since.is_some());

        s.upsert_working("S1".into(), None, pane("%1"));
        assert_eq!(s.sessions["S1"].status, Status::Working);
        assert!(s.sessions["S1"].waiting_since.is_none());
    }

    #[test]
    fn mark_waiting_transitions_and_records_metadata() {
        let mut s = State::default();
        s.upsert_working("S1".into(), None, pane("%1"));

        s.mark_waiting(
            "S1".into(),
            Some("/cwd".into()),
            pane("%1"),
            Some("msg".into()),
        );
        let session = &s.sessions["S1"];
        assert_eq!(session.status, Status::Waiting);
        assert_eq!(session.last_message.as_deref(), Some("msg"));
        assert!(session.waiting_since.is_some());
    }

    #[test]
    fn mark_waiting_keeps_previous_message_when_new_one_is_none() {
        let mut s = State::default();
        s.mark_waiting("S1".into(), None, pane("%1"), Some("first".into()));
        s.mark_waiting("S1".into(), None, pane("%1"), None);
        assert_eq!(s.sessions["S1"].last_message.as_deref(), Some("first"));
    }

    #[test]
    fn drop_session_clears_both_indexes() {
        let mut s = State::default();
        s.upsert_working("S1".into(), None, pane("%1"));
        s.drop_session("S1");
        assert!(s.sessions.is_empty());
        assert!(s.by_pane.is_empty());
    }

    #[test]
    fn visit_pane_clears_only_when_waiting() {
        let mut s = State::default();
        s.upsert_working("S1".into(), None, pane("%1"));
        // Working: visit returns false, no transition.
        assert!(!s.visit_pane("%1"));
        assert_eq!(s.sessions["S1"].status, Status::Working);

        // Waiting: visit returns true, flips to Working.
        s.mark_waiting("S1".into(), None, pane("%1"), None);
        assert!(s.visit_pane("%1"));
        assert_eq!(s.sessions["S1"].status, Status::Working);
        assert!(s.sessions["S1"].waiting_since.is_none());
    }

    #[test]
    fn visit_pane_returns_false_for_unknown_pane() {
        let mut s = State::default();
        s.upsert_working("S1".into(), None, pane("%1"));
        assert!(!s.visit_pane("%99"));
    }

    #[test]
    fn waiting_count_matches_waiting_sessions() {
        let mut s = State::default();
        s.upsert_working("A".into(), None, pane("%1"));
        s.mark_waiting("B".into(), None, pane("%2"), None);
        s.mark_waiting("C".into(), None, pane("%3"), None);
        assert_eq!(s.waiting_count(), 2);
    }

    #[test]
    fn is_idle_only_when_empty_and_threshold_elapsed() {
        let mut s = State::default();
        // Empty + zero threshold = idle.
        assert!(s.is_idle(Duration::from_millis(0)));

        // Non-empty: never idle, regardless of threshold.
        s.upsert_working("A".into(), None, pane("%1"));
        assert!(!s.is_idle(Duration::from_millis(0)));

        // Empty again but with a long threshold = not idle.
        s.drop_session("A");
        assert!(!s.is_idle(Duration::from_secs(3600)));
    }

    #[test]
    fn snapshot_orders_waiting_first() {
        let mut s = State::default();
        s.upsert_working("working".into(), None, pane("%1"));
        s.mark_waiting("waiting".into(), None, pane("%2"), None);
        let snap = s.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].session_id, "waiting");
        assert_eq!(snap[1].session_id, "working");
    }
}
