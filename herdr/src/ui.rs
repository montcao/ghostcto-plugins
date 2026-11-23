//! Layout of the pane, after the GhostCTO website's plate: Big Boss's recap
//! of the watched session on top, and every profile as a small ghost below.
//! Selecting a ghost (click, or arrows and Enter) swaps the recap for what
//! that profile did. When the grid does not fit, it falls back to one line
//! per profile so a small pane still shows every state in words.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use crate::ledger::Report;
use crate::model::{AgentStatus, Board, Ghost, GhostState};
use crate::recap::{Recap, Source};
use crate::sprite;

pub const CARD_WIDTH: u16 = 13;
pub const CARD_HEIGHT: u16 = sprite::MINI_HEIGHT + 2;
const TOP_MIN: u16 = 8;

pub enum RecapState<'a> {
    Off,
    None,
    Shown(&'a Recap),
}

pub struct Detail {
    pub slug: String,
    pub report: Report,
    pub commits: Vec<(String, String)>,
    pub cwd: String,
}

pub struct View<'a> {
    pub board: &'a Board,
    pub watching: Option<(&'a str, &'a str)>,
    pub notice: Option<&'a str>,
    pub frame: usize,
    pub now: u64,
    pub recap: RecapState<'a>,
    pub recap_writing: bool,
    pub recap_error: Option<&'a str>,
    pub selected: Option<usize>,
    pub detail: Option<&'a Detail>,
}

/// Where each ghost was drawn, for mouse clicks and arrow keys.
#[derive(Default)]
pub struct Layout {
    pub cards: Vec<(Rect, usize)>,
    pub per_row: usize,
}

impl Layout {
    pub fn hit(&self, col: u16, row: u16) -> Option<usize> {
        self.cards
            .iter()
            .find(|(r, _)| col >= r.x && col < r.right() && row >= r.y && row < r.bottom())
            .map(|(_, i)| *i)
    }
}

fn state_style(state: GhostState) -> Style {
    match state {
        GhostState::Working => Style::new()
            .fg(Color::Rgb(0xc9, 0xb4, 0xff))
            .add_modifier(Modifier::BOLD),
        GhostState::NeedsYou => Style::new()
            .fg(Color::Rgb(0xff, 0xfa, 0xf2))
            .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        GhostState::Idle => Style::new().add_modifier(Modifier::DIM),
    }
}

fn state_word(ghost: &Ghost) -> String {
    match (ghost.state, ghost.in_subagent) {
        (GhostState::Idle, _) => "idle".into(),
        (state, true) => format!("{} (subagent)", state.word()),
        (state, false) => state.word().into(),
    }
}

fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

pub fn ago(now: u64, ts: u64) -> String {
    let secs = now.saturating_sub(ts);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86400),
    }
}

const HINT: &str = "click or arrows+Enter: what a profile did  r recap  Tab agent  q quit";

pub fn draw(f: &mut Frame, view: &View) -> Layout {
    let mut layout = Layout::default();
    let area = f.area();
    if area.height < 3 || area.width < 10 {
        return layout;
    }
    let mut body = Rect {
        height: area.height - 1,
        ..area
    };
    f.render_widget(
        Paragraph::new(HINT).style(dim()),
        Rect {
            y: area.bottom() - 1,
            height: 1,
            ..area
        },
    );
    if let Some(notice) = view.notice {
        f.render_widget(
            Paragraph::new(notice.to_string()).style(dim()),
            Rect { height: 1, ..body },
        );
        body.y += 1;
        body.height -= 1;
    }

    let ghosts = &view.board.ghosts;
    if ghosts.is_empty() {
        let text = "No GhostCTO profiles found. Install the ghostcto Claude Code plugin, then reopen this pane.";
        f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }), body);
        return layout;
    }

    let per_row = (body.width / CARD_WIDTH).max(1) as usize;
    layout.per_row = per_row;
    let grid_height = ghosts.len().div_ceil(per_row) as u16 * CARD_HEIGHT;
    let grid_fits = body.width >= CARD_WIDTH && grid_height + TOP_MIN < body.height;
    let lower_height = if grid_fits {
        grid_height
    } else {
        (ghosts.len() as u16).min(body.height / 2)
    };
    let top = Rect {
        height: body.height.saturating_sub(lower_height + 1),
        ..body
    };
    let rule = Rect {
        y: top.bottom(),
        height: 1,
        ..body
    };
    let lower = Rect {
        y: rule.bottom(),
        height: body.bottom().saturating_sub(rule.bottom()),
        ..body
    };

    match view
        .detail
        .and_then(|d| view.board.ghost(&d.slug).map(|g| (d, g)))
    {
        Some((detail, ghost)) => draw_detail(f, top, view, detail, ghost),
        None => draw_recap(f, top, view),
    }
    f.render_widget(
        Paragraph::new("─".repeat(body.width as usize)).style(dim()),
        rule,
    );
    if grid_fits {
        draw_cards(f, lower, view, per_row, &mut layout);
    } else {
        draw_list(f, lower, view, &mut layout);
    }
    layout
}

/// Shortens a name to `width` columns, marking the cut.
fn fit(name: &str, width: usize) -> String {
    if name.chars().count() <= width {
        return name.to_string();
    }
    let mut cut: String = name.chars().take(width.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

fn name_of(board: &Board, slug: &str) -> String {
    board
        .ghost(slug)
        .map(|g| g.profile.name.clone())
        .unwrap_or_else(|| slug.to_string())
}

/// Draws a large ghost at the left of `area` and returns the space beside it.
fn with_portrait(
    f: &mut Frame,
    area: Rect,
    ghost: Option<&Ghost>,
    label: &str,
    frame: usize,
) -> Rect {
    if area.width < sprite::WIDTH + 24 || area.height <= sprite::HEIGHT {
        return area;
    }
    let state = ghost.map(|g| g.state).unwrap_or(GhostState::Idle);
    let category = ghost
        .map(|g| g.profile.category)
        .unwrap_or(crate::roster::Category::Build);
    sprite::draw(f.buffer_mut(), area.x, area.y, state, category, frame);
    f.render_widget(
        Paragraph::new(fit(label, sprite::WIDTH as usize))
            .alignment(Alignment::Center)
            .style(dim()),
        Rect {
            y: area.y + sprite::HEIGHT,
            height: 1,
            width: sprite::WIDTH,
            ..area
        },
    );
    Rect {
        x: area.x + sprite::WIDTH + 2,
        width: area.width - sprite::WIDTH - 2,
        ..area
    }
}

/// Big Boss's note on the watched session.
fn recap_lines(view: &View) -> Vec<Line<'static>> {
    let board = view.board;
    let mut lines = Vec::new();
    let (pane, title) = view.watching.unwrap_or(("", ""));
    let title = if title.is_empty() {
        "Claude Code session".to_string()
    } else {
        title.to_string()
    };
    lines.push(Line::from(Span::styled(title, bold())));
    if pane.is_empty() {
        lines.push(Line::from(Span::styled(
            "no Claude Code agent in herdr yet",
            dim(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!("pane {pane} · agent {}", board.agent.word()),
            dim(),
        )));
    }
    lines.push(Line::raw(""));

    let lit: Vec<&Ghost> = board
        .ghosts
        .iter()
        .filter(|g| g.state != GhostState::Idle)
        .collect();
    let mut now = vec![Span::styled("Now  ", dim())];
    if lit.is_empty() {
        now.push(Span::raw(match board.agent {
            AgentStatus::Working => "working, no GhostCTO profile",
            AgentStatus::Blocked => "waiting for you",
            AgentStatus::Idle => "idle",
            AgentStatus::Unknown => "unknown",
        }));
    }
    for (i, ghost) in lit.iter().enumerate() {
        if i > 0 {
            now.push(Span::raw(", "));
        }
        now.push(Span::raw(format!("{} ", ghost.profile.name)));
        now.push(Span::styled(state_word(ghost), state_style(ghost.state)));
    }
    lines.push(Line::from(now));

    match &view.recap {
        RecapState::Shown(recap) => {
            lines.push(Line::from(vec![
                Span::styled("Recap  ", dim()),
                Span::raw(recap.text.clone()),
            ]));
            if !recap.good_for.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("Good for next  ", dim()),
                    Span::raw(recap.good_for.clone()),
                ]));
            }
            let source = match recap.source {
                Source::Claude => "Claude Code's recap",
                Source::Haiku => "recap by Haiku",
            };
            let mut label = format!("{source}, {}", ago(view.now, recap.ts));
            if view.recap_writing {
                label.push_str(" · writing a newer one…");
            }
            lines.push(Line::from(Span::styled(label, dim())));
        }
        RecapState::None => {
            let text = if view.recap_writing {
                "writing one…"
            } else {
                "none yet. One appears when a turn ends; press r to write one now."
            };
            lines.push(Line::from(vec![
                Span::styled("Recap  ", dim()),
                Span::raw(text),
            ]));
        }
        RecapState::Off => lines.push(Line::from(vec![
            Span::styled("Recap  ", dim()),
            Span::raw("off (GHOSTCTO_RECAP=off)"),
        ])),
    }
    if let Some(err) = view.recap_error {
        lines.push(Line::from(Span::styled(
            format!("Recap failed: {err}"),
            dim(),
        )));
    }

    let mut used = vec![Span::styled("Used  ", dim())];
    if board.used.is_empty() {
        used.push(Span::raw(
            "no GhostCTO profile yet. Run /ghostcto:start to plan with Big Boss.",
        ));
    } else {
        let list: Vec<String> = board
            .used
            .iter()
            .map(|(slug, n)| {
                if *n > 1 {
                    format!("{} ×{n}", name_of(board, slug))
                } else {
                    name_of(board, slug)
                }
            })
            .collect();
        used.push(Span::raw(list.join(" · ")));
    }
    lines.push(Line::from(used));
    lines
}

fn draw_recap(f: &mut Frame, area: Rect, view: &View) {
    if area.height == 0 {
        return;
    }
    let text_area = with_portrait(f, area, view.board.ghost("bigboss"), "Big Boss", view.frame);
    f.render_widget(
        Paragraph::new(recap_lines(view)).wrap(Wrap { trim: true }),
        text_area,
    );
}

fn relative<'a>(path: &'a str, cwd: &str) -> &'a str {
    if cwd.is_empty() {
        return path;
    }
    path.strip_prefix(cwd)
        .map(|p| p.trim_start_matches('/'))
        .filter(|p| !p.is_empty())
        .unwrap_or(path)
}

fn counted(map: &std::collections::BTreeMap<String, usize>) -> String {
    map.iter()
        .map(|(k, n)| {
            if *n > 1 {
                format!("{k} ×{n}")
            } else {
                k.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// What one profile did, from recorded events only.
pub fn detail_lines(view: &View, detail: &Detail, ghost: &Ghost) -> Vec<Line<'static>> {
    let r = &detail.report;
    let mut lines = vec![
        Line::from(Span::styled(
            format!("{}: what it did this session", ghost.profile.name),
            bold(),
        )),
        Line::from(Span::styled(
            format!("now {}", state_word(ghost)),
            state_style(ghost.state),
        )),
        Line::raw(""),
    ];
    let row = |label: &str, value: String| {
        Line::from(vec![
            Span::styled(format!("{label}  "), dim()),
            Span::raw(value),
        ])
    };
    if r.starts.is_empty() && r.calls == 0 {
        lines.push(Line::raw("No recorded work in this session."));
        lines.push(Line::from(Span::styled(
            "Work is recorded from when the ghostcto plugin's herdr-ghosts hook is installed.",
            dim(),
        )));
    } else {
        // Events from the first hook version carry no time; count them, but
        // only say when for the ones that do.
        let timed: Vec<u64> = r.starts.iter().copied().filter(|&t| t > 0).collect();
        if !r.starts.is_empty() {
            let when = match (timed.first(), timed.last()) {
                (Some(first), _) if r.starts.len() == 1 => ago(view.now, *first),
                (Some(first), Some(last)) if timed.len() > 1 => {
                    format!(
                        "{}×, first {}, last {}",
                        r.starts.len(),
                        ago(view.now, *first),
                        ago(view.now, *last)
                    )
                }
                (_, Some(last)) => format!("{}×, last {}", r.starts.len(), ago(view.now, *last)),
                _ => format!("{}×", r.starts.len()),
            };
            lines.push(row("Started", when));
        }
        let mut calls = format!("{}", r.calls);
        if r.subagent_calls > 0 {
            calls.push_str(&format!(" ({} in subagents)", r.subagent_calls));
        }
        lines.push(row("Tool calls", calls));
        if !r.edited.is_empty() {
            let files: Vec<&str> = r.edited.iter().map(|p| relative(p, &detail.cwd)).collect();
            lines.push(row(&format!("Edited {}", r.edited.len()), files.join(", ")));
        }
        if !r.read.is_empty() {
            lines.push(row("Read", format!("{} files", r.read.len())));
        }
        if !r.commands.is_empty() {
            lines.push(row("Ran", counted(&r.commands)));
        }
        if !r.delegated.is_empty() {
            lines.push(row("Delegated", counted(&r.delegated)));
        }
        if !r.other_tools.is_empty() {
            lines.push(row("Also", counted(&r.other_tools)));
        }
        for (hash, subject) in &detail.commits {
            lines.push(row("Commit", format!("{hash} {subject}")));
        }
        if !r.commit_times.is_empty() && detail.commits.is_empty() {
            lines.push(row(
                "Commit",
                "ran git commit, but no matching commit was found in this folder".into(),
            ));
        }
    }
    lines.push(Line::from(Span::styled("Esc back", dim())));
    lines
}

fn draw_detail(f: &mut Frame, area: Rect, view: &View, detail: &Detail, ghost: &Ghost) {
    let text_area = with_portrait(f, area, Some(ghost), &ghost.profile.name, view.frame);
    f.render_widget(
        Paragraph::new(detail_lines(view, detail, ghost)).wrap(Wrap { trim: true }),
        text_area,
    );
}

fn draw_cards(f: &mut Frame, area: Rect, view: &View, per_row: usize, layout: &mut Layout) {
    for (i, ghost) in view.board.ghosts.iter().enumerate() {
        let x = area.x + (i % per_row) as u16 * CARD_WIDTH;
        let y = area.y + (i / per_row) as u16 * CARD_HEIGHT;
        if y + CARD_HEIGHT > area.bottom() || x >= area.right() {
            continue;
        }
        let card = Rect {
            x,
            y,
            width: CARD_WIDTH.min(area.right() - x),
            height: CARD_HEIGHT,
        };
        layout.cards.push((card, i));
        sprite::draw_mini(
            f.buffer_mut(),
            x + (card.width.saturating_sub(sprite::MINI_WIDTH)) / 2,
            y,
            ghost.state,
            view.frame,
        );
        let name = fit(&ghost.profile.name, card.width.saturating_sub(1) as usize);
        let mut name_style = if ghost.state == GhostState::Idle {
            dim()
        } else {
            Style::new()
        };
        if view.selected == Some(i) {
            name_style = Style::new().add_modifier(Modifier::REVERSED);
        }
        f.render_widget(
            Paragraph::new(Span::styled(name, name_style)).alignment(Alignment::Center),
            Rect {
                y: y + sprite::MINI_HEIGHT,
                height: 1,
                ..card
            },
        );
        if ghost.state != GhostState::Idle {
            f.render_widget(
                Paragraph::new(Span::styled(state_word(ghost), state_style(ghost.state)))
                    .alignment(Alignment::Center),
                Rect {
                    y: y + sprite::MINI_HEIGHT + 1,
                    height: 1,
                    ..card
                },
            );
        }
    }
}

fn draw_list(f: &mut Frame, area: Rect, view: &View, layout: &mut Layout) {
    let name_width = (area.width as usize).saturating_sub(22).clamp(8, 24);
    let mut order: Vec<usize> = (0..view.board.ghosts.len())
        .filter(|&i| view.board.ghosts[i].state != GhostState::Idle)
        .collect();
    order.extend(
        (0..view.board.ghosts.len()).filter(|&i| view.board.ghosts[i].state == GhostState::Idle),
    );
    layout.per_row = 1;
    let mut lines = Vec::new();
    for (row, &i) in order.iter().enumerate().take(area.height as usize) {
        let g = &view.board.ghosts[i];
        layout.cards.push((
            Rect {
                y: area.y + row as u16,
                height: 1,
                ..area
            },
            i,
        ));
        let name_style = if view.selected == Some(i) {
            Style::new().add_modifier(Modifier::REVERSED)
        } else {
            Style::new()
        };
        lines.push(Line::from(vec![
            Span::styled("ᗣ ", Style::new().fg(sprite::body_colour(g.state))),
            Span::styled(
                format!("{:<w$}", fit(&g.profile.name, name_width), w = name_width),
                name_style,
            ),
            Span::raw(" "),
            Span::styled(state_word(g), state_style(g.state)),
        ]));
    }
    f.render_widget(Paragraph::new(lines), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{Ledger, Report};
    use crate::roster::Profile;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn view<'a>(board: &'a Board, recap: RecapState<'a>, detail: Option<&'a Detail>) -> View<'a> {
        View {
            board,
            watching: Some(("w9:p1", "Ghostcto bigboss profile")),
            notice: None,
            frame: 0,
            now: 1000,
            recap,
            recap_writing: false,
            recap_error: None,
            selected: None,
            detail,
        }
    }

    fn render(width: u16, height: u16, v: &View) -> (Vec<String>, Layout) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut layout = Layout::default();
        terminal.draw(|f| layout = draw(f, v)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let screen = (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        (screen, layout)
    }

    fn board() -> Board {
        let mut b = Board::new(vec![
            Profile::from_slug("bigboss"),
            Profile::from_slug("committer"),
        ]);
        b.agent_status(AgentStatus::Working);
        b.record("ghostcto:start");
        b.skill_started("ghostcto:committer", "");
        b
    }

    #[test]
    fn recap_on_top_and_small_ghosts_below() {
        let b = board();
        let recap = Recap {
            ts: 880,
            text: "Built the pane; next the hook.".into(),
            good_for: "Hook work.".into(),
            source: Source::Haiku,
        };
        let (screen, layout) = render(70, 20, &view(&b, RecapState::Shown(&recap), None));
        let text = screen.join("\n");
        assert!(screen[0].contains("Ghostcto bigboss profile"), "{text}");
        assert!(text.contains("Now  Committer working"), "{text}");
        assert!(
            text.contains("Recap  Built the pane; next the hook."),
            "{text}"
        );
        assert!(text.contains("Good for next  Hook work."), "{text}");
        assert!(text.contains("recap by Haiku, 2 min ago"), "{text}");
        assert!(text.contains("Used  Bigboss · Committer"), "{text}");
        let rule = screen
            .iter()
            .position(|l| l.starts_with("────"))
            .expect("rule");
        assert!(rule >= 20 / 2 - 1, "recap takes the top half: {text}");
        assert!(screen[rule + 4].contains("Committer"), "{text}");
        assert_eq!(layout.cards.len(), 2);
        let (card, idx) = layout.cards[1];
        assert_eq!(layout.hit(card.x + 1, card.y + 1), Some(idx));
        assert_eq!(layout.hit(0, 0), None);
    }

    #[test]
    fn no_recap_yet_says_how_one_appears() {
        let b = board();
        let (screen, _) = render(70, 20, &view(&b, RecapState::None, None));
        assert!(
            screen
                .join(" ")
                .contains("none yet. One appears when a turn ends")
        );
    }

    #[test]
    fn detail_shows_recorded_work_with_paths_relative_to_the_repo() {
        let mut b = board();
        b.agent_status(AgentStatus::Idle);
        let mut l = Ledger::default();
        for json in [
            r#"{"v":1,"pane":"p","skill":"ghostcto:committer"}"#,
            r#"{"v":1,"pane":"p","ts":100,"kind":"skill","skill":"ghostcto:committer"}"#,
            r#"{"v":1,"pane":"p","ts":160,"kind":"skill","skill":"ghostcto:committer"}"#,
            r#"{"v":1,"pane":"p","ts":110,"kind":"tool","tool":"Edit","detail":"/repo/src/a.rs"}"#,
            r#"{"v":1,"pane":"p","ts":120,"kind":"tool","tool":"Bash","detail":"cd, git commit, git push"}"#,
        ] {
            l.apply(&crate::events::parse_line(json).unwrap());
        }
        let detail = Detail {
            slug: "committer".into(),
            report: l.report("committer"),
            commits: vec![("f366862".into(), "feat(profiles): ship a hook".into())],
            cwd: "/repo".into(),
        };
        let (screen, _) = render(80, 22, &view(&b, RecapState::None, Some(&detail)));
        let text = screen.join("\n");
        assert!(
            text.contains("Committer: what it did this session"),
            "{text}"
        );
        assert!(
            text.contains("Started  3×, first 15 min ago, last 14 min ago"),
            "{text}"
        );
        assert!(text.contains("Edited 1  src/a.rs"), "{text}");
        assert!(text.contains("Ran  git commit, git push"), "{text}");
        assert!(
            text.contains("Commit  f366862 feat(profiles): ship a hook"),
            "{text}"
        );
        assert!(!text.contains("Recap"), "detail replaces the recap: {text}");
    }

    #[test]
    fn one_timed_start_among_several_reads_as_last_only() {
        let b = board();
        let mut l = Ledger::default();
        for json in [
            r#"{"v":1,"pane":"p","skill":"ghostcto:committer"}"#,
            r#"{"v":1,"pane":"p","ts":100,"kind":"skill","skill":"ghostcto:committer"}"#,
        ] {
            l.apply(&crate::events::parse_line(json).unwrap());
        }
        let detail = Detail {
            slug: "committer".into(),
            report: l.report("committer"),
            commits: vec![],
            cwd: String::new(),
        };
        let (screen, _) = render(80, 22, &view(&b, RecapState::None, Some(&detail)));
        assert!(
            screen.join("\n").contains("Started  2×, last 15 min ago"),
            "{}",
            screen.join("\n")
        );
    }

    #[test]
    fn detail_without_records_says_so() {
        let b = board();
        let detail = Detail {
            slug: "bigboss".into(),
            report: Report::default(),
            commits: vec![],
            cwd: String::new(),
        };
        let (screen, _) = render(80, 22, &view(&b, RecapState::None, Some(&detail)));
        assert!(
            screen
                .join(" ")
                .contains("No recorded work in this session.")
        );
    }

    #[test]
    fn short_pane_falls_back_to_a_clickable_list_with_lit_first() {
        let b = board();
        let (screen, layout) = render(40, 14, &view(&b, RecapState::None, None));
        let text = screen.join("\n");
        let committer = screen
            .iter()
            .position(|l| l.starts_with("ᗣ Committer"))
            .expect(&text);
        let boss = screen
            .iter()
            .position(|l| l.starts_with("ᗣ Bigboss"))
            .expect(&text);
        assert!(
            committer < boss && screen[committer].ends_with("working"),
            "{text}"
        );
        assert_eq!(layout.hit(3, committer as u16), Some(1));
    }

    #[test]
    fn times_read_as_relative() {
        assert_eq!(ago(1000, 990), "just now");
        assert_eq!(ago(1000, 880), "2 min ago");
        assert_eq!(ago(10_000, 2_000), "2 h ago");
    }

    #[test]
    fn long_names_are_cut_not_run_together() {
        assert_eq!(fit("CI/CD Infrastructure", 10), "CI/CD Inf…");
        assert_eq!(fit("Committer", 10), "Committer");
    }

    #[test]
    fn empty_roster_explains_what_to_install() {
        let b = Board::new(vec![]);
        let (screen, _) = render(40, 6, &view(&b, RecapState::None, None));
        assert!(screen[0].starts_with("No GhostCTO profiles found."));
    }
}
