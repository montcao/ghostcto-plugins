//! GhostCTO ghosts for herdr: a pane that shows which GhostCTO profile a
//! Claude Code agent is using, what each profile did, and a short recap.
//!
//! `ghostcto-herdr open` is the herdr action: it opens the pane beside the
//! focused agent. `ghostcto-herdr pane` is the pane itself. It wakes only for a
//! herdr status change, a new hook event, a key or click, a finished recap, or
//! an animation frame while a ghost is working; when nothing works it sleeps.

mod events;
mod herdr;
mod ledger;
mod model;
mod recap;
mod roster;
mod sprite;
mod ui;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use notify::{RecursiveMode, Watcher};
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use ratatui::crossterm::execute;

use events::Kind;
use ledger::Ledger;
use model::{AgentStatus, Board};
use recap::Recap;

pub enum Msg {
    Status {
        pane: String,
        status: String,
    },
    Resync,
    HerdrGone(String),
    FileChanged,
    Key(KeyCode, KeyModifiers),
    Click(u16, u16),
    Redraw,
    RecapDone {
        session: String,
        result: Result<Recap, String>,
    },
}

const FRAME: Duration = Duration::from_millis(350);

/// Keys and clicks that arrive this soon after the pane opens are dropped:
/// input left over from opening the pane must not open a profile's detail.
const SETTLE: Duration = Duration::from_millis(500);

fn accepts_input(opened: Instant, now: Instant) -> bool {
    now.duration_since(opened) >= SETTLE
}

fn main() {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "pane".to_string());
    let result = match command.as_str() {
        "open" => herdr::open_pane(),
        "pane" => run_pane(),
        "--version" => {
            println!("ghostcto-herdr {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => Err(format!(
            "unknown command {other:?}: expected `open` or `pane`"
        )),
    };
    if let Err(reason) = result {
        eprintln!("ghostcto-herdr: {reason}");
        std::process::exit(1);
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Commits in `cwd` made around the times a profile ran `git commit`.
fn commits_near(cwd: &str, times: &[u64]) -> Vec<(String, String)> {
    if cwd.is_empty() || times.is_empty() {
        return Vec::new();
    }
    let Ok(out) = Command::new("git")
        .args([
            "-C",
            cwd,
            "-c",
            "core.fsmonitor=false",
            "log",
            "-n",
            "200",
            "--format=%h%x09%ct%x09%s",
        ])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let (hash, ct, subject) = (
                parts.next()?,
                parts.next()?.parse::<u64>().ok()?,
                parts.next()?,
            );
            times
                .iter()
                .any(|&t| ct <= t + 5 && ct + 600 >= t)
                .then(|| (hash.to_string(), subject.to_string()))
        })
        .take(10)
        .collect()
}

struct App {
    state_dir: PathBuf,
    events_path: PathBuf,
    board: Board,
    ledger: Ledger,
    agents: Vec<herdr::Agent>,
    watching: Option<usize>,
    notice: Option<String>,
    frame: usize,
    tx: mpsc::Sender<Msg>,
    subscription: Option<herdr::Subscription>,
    subscribed: Vec<String>,
    recap: Option<Recap>,
    claude_recap: Option<Recap>,
    recap_writing: bool,
    recap_error: Option<String>,
    recap_tried_for: Option<u64>,
    selected: Option<usize>,
    detail: Option<ui::Detail>,
    layout: ui::Layout,
}

impl App {
    fn watched(&self) -> Option<&herdr::Agent> {
        self.watching.and_then(|i| self.agents.get(i))
    }

    /// Re-reads herdr's agents, keeping the watched pane when it still exists.
    fn refresh_agents(&mut self, prefer: Option<String>) {
        match herdr::claude_agents() {
            Ok(agents) => {
                self.agents = agents;
                self.notice = None;
            }
            Err(reason) => {
                self.notice = Some(reason);
                return;
            }
        }
        self.resubscribe();
        let keep = prefer.or_else(|| self.watched().map(|a| a.pane_id.clone()));
        self.watching = keep
            .and_then(|pane| self.agents.iter().position(|a| a.pane_id == pane))
            .or(if self.agents.is_empty() {
                None
            } else {
                Some(0)
            });
        let status = self
            .watched()
            .map(|a| AgentStatus::parse(&a.status))
            .unwrap_or(AgentStatus::Unknown);
        self.board.agent_status(status);
    }

    /// Follows the status of every Claude pane, replacing the subscription
    /// only when the set of panes changed.
    fn resubscribe(&mut self) {
        let panes: Vec<String> = self.agents.iter().map(|a| a.pane_id.clone()).collect();
        if panes == self.subscribed && self.subscription.is_some() {
            return;
        }
        self.subscription = None;
        self.subscribed = panes.clone();
        if panes.is_empty() {
            self.notice =
                Some("No Claude Code agent in herdr. Start one, then press Tab.".to_string());
            return;
        }
        match herdr::subscribe(&panes, self.tx.clone()) {
            Ok(sub) => self.subscription = Some(sub),
            Err(reason) => self.notice = Some(reason),
        }
    }

    /// Whether an event belongs to the watched agent: the same herdr pane and,
    /// when both sides know it, the same Claude Code session. A pane can host
    /// a new session later; its old session's events must not light ghosts.
    fn owns(&self, event: &events::Event) -> bool {
        let Some(agent) = self.watched() else {
            return false;
        };
        agent.pane_id == event.pane
            && (agent.session.is_empty()
                || event.session.is_empty()
                || agent.session == event.session)
    }

    /// Rebuilds the watched session's ledger and history from the events
    /// file, without lighting any ghost, and loads its recaps.
    fn load_history(&mut self) {
        let status = self.board.agent;
        self.board.reset(status);
        self.ledger = Ledger::default();
        for event in events::read_all(&self.events_path) {
            if self.owns(&event) {
                self.ledger.apply(&event);
                if event.kind() == Some(Kind::Skill) {
                    self.board.record(&event.skill);
                }
            }
        }
        let session = self
            .watched()
            .map(|a| a.session.clone())
            .unwrap_or_default();
        self.recap = recap::load(&self.state_dir, &session);
        self.recap_error = None;
        self.recap_tried_for = None;
        self.detail = None;
        self.refresh_claude_recap();
        self.maybe_recap(false);
    }

    fn refresh_claude_recap(&mut self) {
        self.claude_recap = self
            .ledger
            .transcript
            .as_deref()
            .and_then(|t| recap::claude_recap(Path::new(t)));
    }

    fn shown_recap(&self) -> Option<&Recap> {
        match (&self.recap, &self.claude_recap) {
            (Some(a), Some(b)) => Some(if b.ts > a.ts { b } else { a }),
            (a, b) => a.as_ref().or(b.as_ref()),
        }
    }

    /// Asks Haiku for a recap when a turn has ended since the newest one, or
    /// at once when `force` (the `r` key).
    fn maybe_recap(&mut self, force: bool) {
        if !recap::enabled() || self.recap_writing {
            return;
        }
        let Some(agent) = self.watched().cloned() else {
            return;
        };
        if agent.session.is_empty() {
            return;
        }
        let last_stop = self.ledger.last_stop;
        if !force {
            let Some(stop) = last_stop else { return };
            if self.board.agent != AgentStatus::Idle
                || self.shown_recap().is_some_and(|r| r.ts >= stop)
                || self.recap_tried_for == Some(stop)
            {
                return;
            }
        }
        let reply_path = self
            .state_dir
            .join("sessions")
            .join(format!("{}.last", agent.session));
        let reply = std::fs::read_to_string(&reply_path)
            .map(|r| recap::decode_reply(&r))
            .unwrap_or_default();
        let previous = self
            .shown_recap()
            .map(|r| r.text.clone())
            .unwrap_or_default();
        if reply.is_empty() && previous.is_empty() {
            if force {
                self.recap_error =
                    Some("nothing to recap yet: no turn has ended with the hook installed".into());
            }
            return;
        }
        let profiles: Vec<String> = self
            .board
            .used
            .iter()
            .map(|(slug, _)| {
                self.board
                    .ghost(slug)
                    .map(|g| g.profile.name.clone())
                    .unwrap_or_else(|| slug.clone())
            })
            .collect();
        let for_ts = last_stop.unwrap_or_else(now);
        self.recap_writing = true;
        self.recap_error = None;
        self.recap_tried_for = last_stop;
        let (tx, dir) = (self.tx.clone(), self.state_dir.clone());
        std::thread::spawn(move || {
            let input = recap::Input {
                title: &agent.title,
                profiles: &profiles.join(", "),
                previous: &previous,
                reply: &reply,
            };
            let result = recap::generate(&input, &dir, for_ts);
            let _ = tx.send(Msg::RecapDone {
                session: agent.session,
                result,
            });
        });
    }

    fn open_detail(&mut self, index: usize) {
        let Some(ghost) = self.board.ghosts.get(index) else {
            return;
        };
        let slug = ghost.profile.slug.clone();
        let report = self.ledger.report(&slug);
        let cwd = self.watched().map(|a| a.cwd.clone()).unwrap_or_default();
        let commits = commits_near(&cwd, &report.commit_times);
        self.selected = Some(index);
        self.detail = Some(ui::Detail {
            slug,
            report,
            commits,
            cwd,
        });
    }

    fn toggle_detail(&mut self, index: usize) {
        let same = self.detail.as_ref().is_some_and(|d| {
            self.board
                .ghosts
                .get(index)
                .is_some_and(|g| g.profile.slug == d.slug)
        });
        if same {
            self.detail = None;
        } else {
            self.open_detail(index);
        }
    }

    fn move_selection(&mut self, delta: isize) {
        let count = self.board.ghosts.len();
        if count == 0 {
            return;
        }
        let current = self.selected.unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, count as isize - 1) as usize;
        self.selected = Some(next);
        if self.detail.is_some() {
            self.open_detail(next);
        }
    }

    fn next_agent(&mut self) {
        self.refresh_agents(None);
        if self.agents.is_empty() {
            return;
        }
        let next = self
            .watching
            .map(|i| (i + 1) % self.agents.len())
            .unwrap_or(0);
        self.watching = Some(next);
        let status = AgentStatus::parse(&self.agents[next].status);
        self.board.reset(status);
        self.load_history();
    }
}

fn claude_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".claude")
}

fn run_pane() -> Result<(), String> {
    herdr::socket_path()?;
    let state_dir = events::state_dir().ok_or("HOME is not set")?;
    recap::create_private_dir(&state_dir).map_err(|e| format!("{}: {e}", state_dir.display()))?;
    let events_path = state_dir.join("events.jsonl");
    let mut tail = events::Tail::at_end(&events_path);

    let (tx, rx) = mpsc::channel::<Msg>();

    let file_tx = tx.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = file_tx.send(Msg::FileChanged);
        }
    })
    .map_err(|e| format!("file watcher: {e}"))?;
    watcher
        .watch(&state_dir, RecursiveMode::NonRecursive)
        .map_err(|e| format!("file watcher: {e}"))?;

    let key_tx = tx.clone();
    std::thread::spawn(move || {
        loop {
            let msg = match event::read() {
                Ok(Event::Key(k)) if k.kind == KeyEventKind::Press => Msg::Key(k.code, k.modifiers),
                Ok(Event::Mouse(m)) if m.kind == MouseEventKind::Down(MouseButton::Left) => {
                    Msg::Click(m.column, m.row)
                }
                Ok(Event::Resize(..)) => Msg::Redraw,
                Ok(_) => continue,
                Err(_) => return,
            };
            if key_tx.send(msg).is_err() {
                return;
            }
        }
    });

    let mut app = App {
        state_dir: state_dir.clone(),
        events_path: events_path.clone(),
        board: Board::new(roster::discover(&claude_home())),
        ledger: Ledger::default(),
        agents: Vec::new(),
        watching: None,
        notice: None,
        frame: 0,
        tx: tx.clone(),
        subscription: None,
        subscribed: Vec::new(),
        recap: None,
        claude_recap: None,
        recap_writing: false,
        recap_error: None,
        recap_tried_for: None,
        selected: None,
        detail: None,
        layout: ui::Layout::default(),
    };
    app.refresh_agents(
        std::env::var("GHOSTCTO_WATCH_PANE")
            .ok()
            .filter(|p| !p.is_empty()),
    );
    app.load_history();

    let opened = Instant::now();
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let outcome = (|| -> Result<(), String> {
        loop {
            let watched = app.watched().map(|a| (a.pane_id.clone(), a.title.clone()));
            let recap_state = if !recap::enabled() {
                ui::RecapState::Off
            } else {
                app.shown_recap()
                    .map(ui::RecapState::Shown)
                    .unwrap_or(ui::RecapState::None)
            };
            let view = ui::View {
                board: &app.board,
                watching: watched.as_ref().map(|(p, t)| (p.as_str(), t.as_str())),
                notice: app.notice.as_deref(),
                frame: app.frame,
                now: now(),
                recap: recap_state,
                recap_writing: app.recap_writing,
                recap_error: app.recap_error.as_deref(),
                selected: app.selected,
                detail: app.detail.as_ref(),
            };
            let mut layout = ui::Layout::default();
            terminal
                .draw(|f| layout = ui::draw(f, &view))
                .map_err(|e| format!("terminal: {e}"))?;
            app.layout = layout;

            let msg = if app.board.animating() {
                match rx.recv_timeout(FRAME) {
                    Ok(msg) => msg,
                    Err(RecvTimeoutError::Timeout) => {
                        app.frame = app.frame.wrapping_add(1);
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return Ok(()),
                }
            } else {
                match rx.recv() {
                    Ok(msg) => msg,
                    Err(_) => return Ok(()),
                }
            };

            let per_row = app.layout.per_row.max(1) as isize;
            if matches!(msg, Msg::Key(..) | Msg::Click(..))
                && !accepts_input(opened, Instant::now())
            {
                continue;
            }
            match msg {
                Msg::Key(KeyCode::Char('q'), _) => return Ok(()),
                Msg::Key(KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                Msg::Key(KeyCode::Esc, _) => {
                    // Esc steps back but never closes the pane; q and Ctrl+C do.
                    if app.detail.is_some() {
                        app.detail = None;
                    } else {
                        app.selected = None;
                    }
                }
                Msg::Key(KeyCode::Tab, _) => app.next_agent(),
                Msg::Key(KeyCode::Left | KeyCode::Char('h'), _) => app.move_selection(-1),
                Msg::Key(KeyCode::Right | KeyCode::Char('l'), _) => app.move_selection(1),
                Msg::Key(KeyCode::Up | KeyCode::Char('k'), _) => app.move_selection(-per_row),
                Msg::Key(KeyCode::Down | KeyCode::Char('j'), _) => app.move_selection(per_row),
                Msg::Key(KeyCode::Enter | KeyCode::Char(' '), _) => {
                    app.toggle_detail(app.selected.unwrap_or(0))
                }
                Msg::Key(KeyCode::Char('r'), _) => app.maybe_recap(true),
                Msg::Key(..) | Msg::Redraw => {}
                Msg::Click(col, row) => {
                    if let Some(index) = app.layout.hit(col, row) {
                        app.toggle_detail(index);
                    }
                }
                Msg::Status { pane, status } => {
                    if let Some(agent) = app.agents.iter_mut().find(|a| a.pane_id == pane) {
                        agent.status = status.clone();
                    }
                    if app.watched().is_some_and(|a| a.pane_id == pane) {
                        let status = AgentStatus::parse(&status);
                        app.board.agent_status(status);
                        if status == AgentStatus::Idle {
                            app.maybe_recap(false);
                        }
                    } else if app.watched().is_none() {
                        app.refresh_agents(None);
                    }
                }
                Msg::Resync => app.refresh_agents(None),
                Msg::HerdrGone(reason) => {
                    app.subscription = None;
                    app.notice = Some(format!("{reason}. Press Tab to reconnect."));
                }
                Msg::FileChanged => {
                    let mut stopped = false;
                    for event in tail.read_new() {
                        if !app.owns(&event) {
                            continue;
                        }
                        app.ledger.apply(&event);
                        match event.kind() {
                            Some(Kind::Skill) => {
                                app.board.skill_started(&event.skill, &event.agent);
                            }
                            Some(Kind::Stop) => stopped = true,
                            _ => {}
                        }
                    }
                    if stopped {
                        app.refresh_claude_recap();
                        app.maybe_recap(false);
                    }
                    if let Some(index) = app.selected.filter(|_| app.detail.is_some()) {
                        app.open_detail(index);
                    }
                }
                Msg::RecapDone { session, result } => {
                    app.recap_writing = false;
                    if app.watched().is_some_and(|a| a.session == session) {
                        match result {
                            Ok(recap) => {
                                recap::save(&app.state_dir, &session, &recap);
                                app.recap = Some(recap);
                                app.recap_error = None;
                            }
                            Err(reason) => app.recap_error = Some(reason),
                        }
                    }
                }
            }
        }
    })();
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    drop(watcher);
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_is_dropped_only_while_the_pane_settles() {
        let opened = Instant::now();
        assert!(!accepts_input(opened, opened + Duration::from_millis(100)));
        assert!(accepts_input(opened, opened + SETTLE));
    }
}
