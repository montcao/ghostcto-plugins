# GhostCTO ghosts for herdr

A [herdr](https://herdr.dev) pane that shows which GhostCTO profile a Claude
Code agent is using and what it did. Every installed profile is a small ghost;
the one the agent is using reads **working**, or **needs you** while herdr
reports the agent blocked, and goes back to **idle** when the turn ends.

- **Recap.** Above the ghosts, Big Boss shows a short recap of the session:
  what it set out to do, where it stands, the next step, and what its loaded
  context is good for next. It is Claude Code's own recap (the one `/recap`
  writes) when that is newest, otherwise one Haiku writes in the same style
  when a turn ends, while this pane is open. Each recap says which wrote it
  and when. Press `r` to write one now; set `GHOSTCTO_RECAP=off` to turn
  Haiku recaps off.
- **What each profile did.** Click a ghost, or use the arrow keys and Enter,
  to see that profile's work in the session: when it started, files it
  edited, files it read, commands it ran, subagents it delegated to, and the
  commits it made. These are recorded facts, not a model's summary. `Esc`
  goes back.

## Install

You need herdr 0.9.5 or newer, the `ghostcto` Claude Code plugin, and
[Rust](https://rustup.rs) to build the pane.

```bash
herdr plugin install montcao/ghostcto-plugins/herdr
```

Then, in a pane running Claude Code, run the action **GhostCTO: show ghosts
for this agent** (`herdr plugin action invoke ghosts --plugin ghostcto`, or
bind it to a key such as `prefix+alt+g`). The ghosts open in a split beside
that agent. Press `Tab` to watch another Claude Code agent and `q` to close.

## How it knows

- **The hook.** The `ghostcto` Claude Code plugin ships a hook,
  `herdr-ghosts`. Outside a herdr pane it does nothing. Inside one it writes to
  `${XDG_STATE_HOME:-~/.local/state}/ghostcto/herdr/`, readable only by you:
  - `events.jsonl`: one line per event with the pane, session, subagent id and
    time, plus the GhostCTO skill started, or a tool's name and one detail (a
    file path, a subagent type, or the program and subcommand of each shell
    command, never its arguments), or a turn end with the session log's path.
    Rotated at 256 KB.
  - `sessions/<session>.last`: Claude's latest reply, cut to 4 KB with common
    token formats removed, for the recap. Deleted when the session ends.
- **Whether the agent is working:** herdr's agent status, pushed over its
  socket (`events.subscribe`, `pane.agent_status_changed`).
- **Whose events:** only lines from the watched pane and the Claude Code
  session herdr reports for it count.
- **Who did what:** each tool call is credited to the profile the same agent
  started last in the turn. A subagent with no profile of its own works for
  the one that delegated to it.
- **Recaps:** Claude Code's recap is read from the session log, whose format
  is internal to Claude Code; when it cannot be read the pane falls back to
  its own. Haiku recaps run `claude -p --model haiku` with no tools, no
  settings and no MCP servers, and are stored in `recaps/<session>.json`.

The pane redraws only when something changes, and animates only while a ghost
is working.

## Limits

- In the main agent one profile reads working at a time: the one started
  last in the turn. A subagent's profile reads "working (subagent)".
- Each Haiku recap is a model call on your Claude plan.
- If herdr misses the end of a turn, a ghost stays lit until the next status
  change.
- Codex and other agents have no GhostCTO hook, so they show no ghosts.

## Develop

```bash
cargo test
cargo clippy --all-targets
sh scripts/plugin-build          # builds bin/ghostcto-herdr
herdr plugin link "$PWD"          # register this checkout with herdr
```

Licensed under Apache-2.0.
