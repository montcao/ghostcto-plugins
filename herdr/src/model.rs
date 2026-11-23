//! What each ghost is doing, decided from two signals: a skill-start event from
//! the GhostCTO hook, and the watched agent's status from herdr.
//!
//! A ghost turns on when its skill starts and stays on until herdr reports the
//! agent idle, which is the end of the turn. While herdr reports the agent
//! blocked, a lit ghost reads "needs you" instead of "working".

use crate::roster::{Profile, slug_for_skill};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Unknown,
}

impl AgentStatus {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "idle" | "done" => AgentStatus::Idle,
            "working" => AgentStatus::Working,
            "blocked" => AgentStatus::Blocked,
            _ => AgentStatus::Unknown,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            AgentStatus::Idle => "idle",
            AgentStatus::Working => "working",
            AgentStatus::Blocked => "blocked",
            AgentStatus::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhostState {
    Idle,
    Working,
    NeedsYou,
}

impl GhostState {
    pub fn word(self) -> &'static str {
        match self {
            GhostState::Idle => "idle",
            GhostState::Working => "working",
            GhostState::NeedsYou => "needs you",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Ghost {
    pub profile: Profile,
    pub state: GhostState,
    /// Lit by a subagent rather than the main agent.
    pub in_subagent: bool,
}

#[derive(Debug)]
pub struct Board {
    pub ghosts: Vec<Ghost>,
    pub agent: AgentStatus,
    /// Profiles this session used, in order of first use, with how many times.
    pub used: Vec<(String, usize)>,
    /// The profile used most recently in this session.
    pub last: Option<String>,
}

impl Board {
    pub fn new(profiles: Vec<Profile>) -> Self {
        Board {
            ghosts: profiles
                .into_iter()
                .map(|profile| Ghost {
                    profile,
                    state: GhostState::Idle,
                    in_subagent: false,
                })
                .collect(),
            agent: AgentStatus::Unknown,
            used: Vec::new(),
            last: None,
        }
    }

    /// Counts a skill use without lighting a ghost: for the session's earlier
    /// turns, read when the pane starts watching it.
    pub fn record(&mut self, skill: &str) -> Option<String> {
        let slug = slug_for_skill(skill)?;
        if !self.ghosts.iter().any(|g| g.profile.slug == slug) {
            self.ghosts.push(Ghost {
                profile: Profile::from_slug(&slug),
                state: GhostState::Idle,
                in_subagent: false,
            });
        }
        match self.used.iter_mut().find(|(s, _)| *s == slug) {
            Some((_, n)) => *n += 1,
            None => self.used.push((slug.clone(), 1)),
        }
        self.last = Some(slug.clone());
        Some(slug)
    }

    pub fn ghost(&self, slug: &str) -> Option<&Ghost> {
        self.ghosts.iter().find(|g| g.profile.slug == slug)
    }

    /// Lights the ghost for `skill`. Returns false for a skill that is not a
    /// GhostCTO profile. A profile missing from the roster is added: the event
    /// proves it is installed somewhere the roster did not look.
    /// Lights the ghost for `skill`, started by the main agent (`agent` empty)
    /// or by a subagent. One profile works at a time in the main agent: the
    /// one started last, so a hand-off from Big Boss to Committer moves the
    /// light. A subagent's profile lights alongside without dimming others.
    /// Returns false for a skill that is not a GhostCTO profile.
    pub fn skill_started(&mut self, skill: &str, agent: &str) -> bool {
        let Some(slug) = self.record(skill) else {
            return false;
        };
        let lit = match self.agent {
            AgentStatus::Blocked => GhostState::NeedsYou,
            _ => GhostState::Working,
        };
        let in_subagent = !agent.is_empty();
        for ghost in &mut self.ghosts {
            if ghost.profile.slug == slug {
                ghost.state = lit;
                ghost.in_subagent = in_subagent;
            } else if !in_subagent && !ghost.in_subagent {
                ghost.state = GhostState::Idle;
            }
        }
        true
    }

    pub fn agent_status(&mut self, status: AgentStatus) {
        self.agent = status;
        for ghost in &mut self.ghosts {
            ghost.state = match (status, ghost.state) {
                (_, GhostState::Idle) => GhostState::Idle,
                (AgentStatus::Idle, _) => GhostState::Idle,
                (AgentStatus::Blocked, _) => GhostState::NeedsYou,
                (AgentStatus::Working, _) => GhostState::Working,
                (AgentStatus::Unknown, state) => state,
            };
        }
    }

    /// Clears every ghost, for when the pane starts watching another agent.
    pub fn reset(&mut self, status: AgentStatus) {
        for ghost in &mut self.ghosts {
            ghost.state = GhostState::Idle;
            ghost.in_subagent = false;
        }
        self.agent = status;
        self.used.clear();
        self.last = None;
    }

    /// Whether anything moves; the pane redraws on a timer only while true.
    pub fn animating(&self) -> bool {
        self.ghosts.iter().any(|g| g.state == GhostState::Working)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> Board {
        let mut b = Board::new(vec![
            Profile::from_slug("bigboss"),
            Profile::from_slug("committer"),
        ]);
        b.agent_status(AgentStatus::Working);
        b
    }

    fn state(b: &Board, slug: &str) -> GhostState {
        b.ghosts
            .iter()
            .find(|g| g.profile.slug == slug)
            .unwrap()
            .state
    }

    #[test]
    fn skill_start_lights_only_its_ghost() {
        let mut b = board();
        assert!(b.skill_started("ghostcto:committer", ""));
        assert_eq!(state(&b, "committer"), GhostState::Working);
        assert_eq!(state(&b, "bigboss"), GhostState::Idle);
        assert!(b.animating());
    }

    #[test]
    fn plugin_start_skill_is_big_boss() {
        let mut b = board();
        b.skill_started("ghostcto:start", "");
        assert_eq!(state(&b, "bigboss"), GhostState::Working);
    }

    #[test]
    fn the_profile_started_last_is_the_one_working() {
        let mut b = board();
        b.skill_started("ghostcto:start", "");
        b.skill_started("ghostcto:committer", "");
        assert_eq!(state(&b, "bigboss"), GhostState::Idle);
        assert_eq!(state(&b, "committer"), GhostState::Working);
    }

    #[test]
    fn a_subagent_profile_lights_beside_the_main_one() {
        let mut b = board();
        b.skill_started("ghostcto:start", "");
        b.skill_started("ghostcto:committer", "ag-1");
        assert_eq!(state(&b, "bigboss"), GhostState::Working);
        assert_eq!(state(&b, "committer"), GhostState::Working);
        assert!(b.ghost("committer").unwrap().in_subagent);
        b.agent_status(AgentStatus::Idle);
        assert!(!b.animating());
    }

    #[test]
    fn idle_agent_turns_every_ghost_off() {
        let mut b = board();
        b.skill_started("ghostcto:start", "");
        b.skill_started("ghostcto-committer", "");
        b.agent_status(AgentStatus::Idle);
        assert!(b.ghosts.iter().all(|g| g.state == GhostState::Idle));
        assert!(!b.animating());
    }

    #[test]
    fn blocked_agent_reads_needs_you_and_recovers() {
        let mut b = board();
        b.skill_started("ghostcto:committer", "");
        b.agent_status(AgentStatus::Blocked);
        assert_eq!(state(&b, "committer"), GhostState::NeedsYou);
        assert_eq!(state(&b, "bigboss"), GhostState::Idle);
        b.agent_status(AgentStatus::Working);
        assert_eq!(state(&b, "committer"), GhostState::Working);
    }

    #[test]
    fn skill_started_while_blocked_needs_you() {
        let mut b = board();
        b.agent_status(AgentStatus::Blocked);
        b.skill_started("ghostcto:committer", "");
        assert_eq!(state(&b, "committer"), GhostState::NeedsYou);
    }

    #[test]
    fn unknown_status_keeps_state() {
        let mut b = board();
        b.skill_started("ghostcto:committer", "");
        b.agent_status(AgentStatus::Unknown);
        assert_eq!(state(&b, "committer"), GhostState::Working);
    }

    #[test]
    fn other_skills_are_ignored() {
        let mut b = board();
        assert!(!b.skill_started("caveman:caveman", ""));
        assert!(!b.skill_started("ghostcto:", ""));
        assert!(b.ghosts.iter().all(|g| g.state == GhostState::Idle));
    }

    #[test]
    fn profile_missing_from_roster_is_added() {
        let mut b = board();
        assert!(b.skill_started("ghostcto:web-designer", ""));
        assert_eq!(state(&b, "web-designer"), GhostState::Working);
    }

    #[test]
    fn history_counts_without_lighting() {
        let mut b = board();
        b.record("ghostcto:start");
        b.record("ghostcto:committer");
        b.record("ghostcto:start");
        assert_eq!(
            b.used,
            vec![("bigboss".to_string(), 2), ("committer".to_string(), 1)]
        );
        assert_eq!(b.last.as_deref(), Some("bigboss"));
        assert!(!b.animating());
        b.skill_started("ghostcto:committer", "");
        assert_eq!(b.used[1].1, 2);
        assert_eq!(b.last.as_deref(), Some("committer"));
    }

    #[test]
    fn reset_clears_ghosts() {
        let mut b = board();
        b.skill_started("ghostcto:committer", "");
        b.reset(AgentStatus::Idle);
        assert!(!b.animating());
        assert_eq!(b.agent, AgentStatus::Idle);
    }
}
