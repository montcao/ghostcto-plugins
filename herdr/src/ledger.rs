//! What each profile did in the watched session, from the hook's events.
//!
//! Every tool call is credited to the profile active in the same agent when
//! it ran: the GhostCTO skill that agent started last in the current turn. A
//! subagent with no skill of its own works for the profile that delegated to
//! it. A turn end (a `stop` event) clears the active profiles. Calls made with
//! no profile active are not credited to anyone.

use std::collections::{BTreeMap, HashMap};

use crate::events::{Event, Kind};
use crate::roster::slug_for_skill;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    pub ts: u64,
    pub tool: String,
    pub detail: String,
    pub in_subagent: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ProfileLog {
    pub starts: Vec<u64>,
    pub activity: Vec<Activity>,
}

#[derive(Debug, Default)]
pub struct Ledger {
    active: HashMap<String, String>,
    pub profiles: HashMap<String, ProfileLog>,
    /// Time of the last turn end seen, for knowing when a recap is stale.
    pub last_stop: Option<u64>,
    /// Session log path from the last turn end, where Claude Code's own
    /// recaps are kept.
    pub transcript: Option<String>,
}

impl Ledger {
    pub fn apply(&mut self, event: &Event) {
        match event.kind() {
            Some(Kind::Skill) => {
                if let Some(slug) = slug_for_skill(&event.skill) {
                    self.profiles
                        .entry(slug.clone())
                        .or_default()
                        .starts
                        .push(event.ts);
                    self.active.insert(event.agent.clone(), slug);
                }
            }
            Some(Kind::Tool) => {
                if event.tool == "Skill" {
                    return;
                }
                let owner = self.active.get(&event.agent).or_else(|| {
                    if event.agent.is_empty() {
                        None
                    } else {
                        self.active.get("")
                    }
                });
                if let Some(slug) = owner.cloned() {
                    self.profiles
                        .entry(slug)
                        .or_default()
                        .activity
                        .push(Activity {
                            ts: event.ts,
                            tool: event.tool.clone(),
                            detail: event.detail.clone(),
                            in_subagent: !event.agent.is_empty(),
                        });
                }
            }
            Some(Kind::Stop) => {
                self.active.clear();
                self.last_stop = Some(event.ts);
                if !event.transcript.is_empty() {
                    self.transcript = Some(event.transcript.clone());
                }
            }
            None => {}
        }
    }

    pub fn report(&self, slug: &str) -> Report {
        let mut report = Report::default();
        let Some(log) = self.profiles.get(slug) else {
            return report;
        };
        report.starts = log.starts.clone();
        for a in &log.activity {
            report.calls += 1;
            if a.in_subagent {
                report.subagent_calls += 1;
            }
            match a.tool.as_str() {
                "Edit" | "Write" | "MultiEdit" | "NotebookEdit" if !a.detail.is_empty() => {
                    if !report.edited.contains(&a.detail) {
                        report.edited.push(a.detail.clone());
                    }
                }
                "Read" if !a.detail.is_empty() => {
                    if !report.read.contains(&a.detail) {
                        report.read.push(a.detail.clone());
                    }
                }
                "Bash" => {
                    for program in a.detail.split(", ").filter(|p| !p.is_empty() && *p != "cd") {
                        *report.commands.entry(program.to_string()).or_default() += 1;
                        if program == "git commit" {
                            report.commit_times.push(a.ts);
                        }
                    }
                }
                "Agent" | "Task" => {
                    let kind = if a.detail.is_empty() {
                        "subagent".to_string()
                    } else {
                        a.detail.clone()
                    };
                    *report.delegated.entry(kind).or_default() += 1;
                }
                other => *report.other_tools.entry(other.to_string()).or_default() += 1,
            }
        }
        report
    }
}

/// One profile's work in the session, as recorded facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub starts: Vec<u64>,
    pub calls: usize,
    pub subagent_calls: usize,
    pub edited: Vec<String>,
    pub read: Vec<String>,
    pub commands: BTreeMap<String, usize>,
    pub delegated: BTreeMap<String, usize>,
    pub other_tools: BTreeMap<String, usize>,
    pub commit_times: Vec<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(json: &str) -> Event {
        crate::events::parse_line(json).unwrap()
    }

    fn skill(agent: &str, ts: u64, skill: &str) -> Event {
        ev(&format!(
            r#"{{"v":1,"pane":"p","session":"s","agent":"{agent}","ts":{ts},"kind":"skill","skill":"{skill}"}}"#
        ))
    }

    fn tool(agent: &str, ts: u64, tool: &str, detail: &str) -> Event {
        ev(&format!(
            r#"{{"v":1,"pane":"p","session":"s","agent":"{agent}","ts":{ts},"kind":"tool","tool":"{tool}","detail":"{detail}"}}"#
        ))
    }

    fn stop(ts: u64) -> Event {
        ev(&format!(
            r#"{{"v":1,"pane":"p","session":"s","agent":"","ts":{ts},"kind":"stop","transcript":"/t.jsonl"}}"#
        ))
    }

    #[test]
    fn credits_tools_to_the_active_profile_and_stops_at_turn_end() {
        let mut l = Ledger::default();
        l.apply(&tool("", 1, "Read", "/before.md"));
        l.apply(&skill("", 2, "ghostcto:start"));
        l.apply(&tool("", 3, "Read", "/plan.md"));
        l.apply(&skill("", 4, "ghostcto:committer"));
        l.apply(&tool("", 5, "Bash", "cd, git commit, git push"));
        l.apply(&tool("", 6, "Skill", "ghostcto:committer"));
        l.apply(&stop(7));
        l.apply(&tool("", 8, "Edit", "/after.rs"));

        let boss = l.report("bigboss");
        assert_eq!(boss.read, vec!["/plan.md"]);
        assert_eq!(boss.calls, 1);
        let committer = l.report("committer");
        assert_eq!(committer.commands.get("git commit"), Some(&1));
        assert_eq!(committer.commands.get("git push"), Some(&1));
        assert!(!committer.commands.contains_key("cd"));
        assert_eq!(committer.commit_times, vec![5]);
        assert!(
            committer.edited.is_empty(),
            "work after the turn ended is nobody's"
        );
        assert_eq!(l.last_stop, Some(7));
        assert_eq!(l.transcript.as_deref(), Some("/t.jsonl"));
    }

    #[test]
    fn subagents_work_for_their_own_skill_or_for_the_profile_that_delegated() {
        let mut l = Ledger::default();
        l.apply(&skill("", 1, "ghostcto:start"));
        l.apply(&tool("", 2, "Agent", "Explore"));
        l.apply(&tool("ag-1", 3, "Read", "/a.rs"));
        l.apply(&skill("ag-2", 4, "ghostcto:security"));
        l.apply(&tool("ag-2", 5, "Read", "/b.rs"));
        let boss = l.report("bigboss");
        assert_eq!(boss.delegated.get("Explore"), Some(&1));
        assert_eq!(boss.read, vec!["/a.rs"]);
        assert_eq!(boss.subagent_calls, 1);
        assert_eq!(l.report("security").read, vec!["/b.rs"]);
    }

    #[test]
    fn edits_are_listed_once_and_unknown_profiles_are_empty() {
        let mut l = Ledger::default();
        l.apply(&skill("", 1, "ghostcto:developer"));
        l.apply(&tool("", 2, "Edit", "/x.rs"));
        l.apply(&tool("", 3, "Edit", "/x.rs"));
        l.apply(&tool("", 4, "Write", "/y.rs"));
        l.apply(&tool("", 5, "Grep", ""));
        let r = l.report("developer");
        assert_eq!(r.edited, vec!["/x.rs", "/y.rs"]);
        assert_eq!(r.other_tools.get("Grep"), Some(&1));
        assert_eq!(r.starts, vec![1]);
        assert_eq!(l.report("lint-fixer"), Report::default());
    }
}
