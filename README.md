# GhostCTO plugins

Plugins that bring [GhostCTO](https://github.com/montcao/ghostcto-profiles)
profiles into the tools people run their coding agents in. Each tool has its
own folder.

| Folder | Tool | Install |
| --- | --- | --- |
| [`herdr/`](herdr/README.md) | [herdr](https://herdr.dev) | `herdr plugin install montcao/ghostcto-plugins/herdr` |

The profiles themselves, and the Claude Code plugin, live in
[ghostcto-profiles](https://github.com/montcao/ghostcto-profiles).

Code lands here only through pull requests with passing checks: formatting,
lints and tests on Linux and macOS, and a supply-chain check of every
dependency's advisories, licence and source (`deny.toml`). See
[SECURITY.md](SECURITY.md) to report a vulnerability.

Licensed under Apache-2.0.
