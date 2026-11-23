# Security policy

## Reporting a vulnerability

Report vulnerabilities privately through GitHub:
[Report a vulnerability](https://github.com/montcao/ghostcto-plugins/security/advisories/new).
Do not open a public issue, pull request or discussion for a suspected
vulnerability.

Include the affected plugin and version or commit, what an attacker could do,
and the steps to reproduce it. We will acknowledge the report, keep you
informed while we investigate, and credit you in the advisory unless you ask
us not to.

## Supported versions

Only the latest commit on `main` receives security fixes.

## Scope

In scope: the code in this repository, its build scripts and its CI workflows.

The Claude Code hook that feeds these plugins (`herdr-ghosts`) ships in the
`ghostcto` Claude Code plugin; report issues with it at
[montcao/ghostcto-profiles](https://github.com/montcao/ghostcto-profiles/security/advisories/new).
Issues in herdr, Claude Code or other host tools belong with those projects.

## How this repository is protected

- Changes reach `main` only through pull requests with required, passing checks;
  force pushes and branch deletion are blocked, and the rules apply to
  administrators.
- Secret scanning and push protection, Dependabot alerts and security updates,
  and code scanning are enabled.
- Workflows run with a read-only token and may use only GitHub-owned actions
  pinned to a full commit SHA.
