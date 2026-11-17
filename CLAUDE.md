# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:6cd5cc61 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->


## Build & Test

```bash
make check        # the gate CI enforces: fmt, clippy, tests, and a wasm32 compile check
make run          # desktop version: a bare CP/M session (Ctrl-\ opens the monitor; q quits)
make setup        # once: download the classic software library (desktop)
make play         # desktop version with the whole library on drive A:
make web-serve    # browser version, built and served at http://localhost:8000/
make web-smoke    # browser version, built and tested in Node (what the deploy runs)
cargo test -p cpemu-cpu --release --test exercisers -- --ignored   # the Z80 exercisers (zexdoc)
```

A push to `main` deploys the browser version to https://cpemulator.degel.com/ (via
`.github/workflows/pages.yml`).

## Architecture Overview

A Z80 CP/M 2.2 emulator in Rust that boots the genuine Digital Research CCP and BDOS
through a high-level-emulated BIOS. One emulator library serves two front ends: the
**desktop** CLI, in a host terminal, and the **browser** version, compiled to
WebAssembly and live at https://cpemulator.degel.com/. Crates: `crates/cpu` (the
Z80), `crates/cpemu` (the system layer and the CLI), `crates/web` (the browser
wrapper); the web page is `web/www`. Start with `dev-docs/README.md`; the browser
build is `dev-docs/browser.md`.

## Conventions & Patterns

- Docs are part of every task: `README.md` for users (covering both the desktop and
  browser versions), `dev-docs/` for developers. Favor clarity over brevity.
- One logical change per commit, naming its bead in the subject.
- This repository is public, and so are its beads and their memories: keep them,
  like the docs and commit messages, to engineering (see `~/CLAUDE.md`, "A Public
  Repo's Beads Are Public").
- The web page's wording rules are in `dev-docs/browser.md`: Get / Add files bring
  things in, Save disk / Load disk move the disk; no "Download" and no ellipsis on
  any button.
