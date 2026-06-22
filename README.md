<div align="center">

# 🔍 Loupe

**Read a pull request the way the code runs — not the way `git diff` prints it.**

Loupe is a macOS desktop app that reorders a PR into **data-flow order** and walks you
through it **one symbol at a time**, with AI that clusters the change, labels each group,
and answers questions about your *actual* codebase inline.

[![Release](https://img.shields.io/github/v/release/JiwonKKang/loupe)](https://github.com/JiwonKKang/loupe/releases)
![Platform](https://img.shields.io/badge/platform-macOS-black)
![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB)

<!-- 👉 Add a screenshot or GIF here, e.g.  ![Loupe](docs/loupe.png) -->

</div>

---

## Why

A diff is a pile of files in alphabetical order. But that's not how you *understand* a
change — you follow it: where data enters, how it's transformed, where it lands. Loupe
does that reordering for you:

- **Data-flow order, not file order.** The engine groups the change into clusters and
  lays them out so you read entry points → core logic → edges.
- **One symbol-card at a time.** A focused card per changed function/type. Keyboard-first:
  `Space` to pass, `←/→` to move. No 4,000-line scroll.
- **Grounded AI, not vibes.** Threads run an agentic, **read-only** Claude scoped to the
  repo — it reads the real definitions and callers, not just the diff snippet.

## Features

- 🧭 **Data-flow review** — clusters + flow ordering so a PR reads as a story.
- 🃏 **Symbol cards** — one changed function/type per card, split before/after diff.
- 💬 **Inline threads** — drag to select lines, ask a question; the answer is
  **codebase-aware** (reads definitions/callers across the repo). Pick Sonnet or Haiku
  per thread. Threads persist with the review.
- 🔗 **Jump links** — when an answer references another changed symbol, it's a click away.
- 🧠 **Editor handoff** — `⌘-click` any diff line to open the **whole project** at that
  `file:line` in **IntelliJ IDEA** or **VS Code**.
- ⌨️ **`loupe` CLI** — `loupe <path> [base] [target]` from a terminal or an AI skill;
  the app opens and starts the review (and remembers it in recents).
- 🔎 **Fast project switching** — branch picker with live search; recent projects.
- 🔍 **`⌘ + scroll`** to zoom the code; adaptive font that fits the change.
- ✅ **Approve the PR** — when a review ends all-pass, approve the GitHub PR straight
  from the summary screen (delegated to your `gh` CLI; explicit two-step confirm).
- 🗄️ **SHA-cached** — re-opening an unchanged range is instant (no AI re-spend).

## Requirements

- **macOS 11 (Big Sur) or later** — universal build (Apple Silicon + Intel).
- **[Claude Code](https://www.anthropic.com/claude-code) CLI** installed and on your `PATH`
  — Loupe shells out to `claude` for all AI work.
- A **Claude setup-token**: run `claude setup-token` and paste it into Loupe once
  (stored locally on your Mac, `chmod 600`, never logged).
- *(optional)* the **[GitHub CLI](https://cli.github.com) (`gh`)**, signed in
  (`gh auth login`) — only needed to approve a PR from the summary screen.

> Loupe makes **no network calls of its own** — every AI request goes through *your*
> `claude` CLI with *your* token.

## Install

**Homebrew (recommended)**

```sh
brew install --cask JiwonKKang/loupe/loupe
```

To update: `brew upgrade --cask loupe`.

**Or grab the DMG** from [Releases](https://github.com/JiwonKKang/loupe/releases) →
drag `Loupe.app` to `/Applications`.

> The build is ad-hoc signed (no paid Apple Developer cert). The Homebrew cask strips the
> quarantine flag for you. If you install the DMG by hand and Gatekeeper complains:
> `xattr -dr com.apple.quarantine /Applications/Loupe.app`.

## Quick start

1. Launch Loupe → paste your `claude setup-token`.
2. Top-left menu → **Browse** to a git repo → pick **base** and **target** branches → **Open review**.
3. Walk the cards: `Space` pass · `←/→` move · `⌘E` jump back to the previous card.
4. Drag across lines on a card to open an inline thread and ask away.
5. `⌘-click` a line to open it in your editor; the bottom bar's logo picks IntelliJ / VS Code.

### `loupe` CLI

Installed alongside the app by the Homebrew cask.

```sh
loupe                                   # current dir, auto base/target
loupe ~/code/myproject main feature/x   # explicit
```

It resolves the range and opens the app straight into the review.

## How it works

```
git 3-dot diff (base...target)
        │
        ▼
tree-sitter symbol extraction        → one card per changed function / type
        │
        ▼
AI clustering + flow ordering (Claude Sonnet)   ┐
per-cluster labels (Claude Haiku, in parallel)  ┘ → data-flow ordered review
        │
        ▼
SHA cache (SQLite)  → unchanged range = instant, 0 AI calls
```

Inline threads run a separate **agentic, read-only** Claude (Read/Grep/Glob/LS only;
no writes, no shell) confined to the repo, so answers are grounded in the real code.

### Language support

| Language | Review granularity |
|---|---|
| **Go, Java, Rust** | Symbol-level (per function / type, via tree-sitter) |
| Everything else | File-level cards (still ordered & clustered) |

Adding a language is mostly wiring up its tree-sitter grammar — PRs welcome.

## Tech stack

- **[Tauri 2](https://tauri.app)** shell · **React 19** (plain JS) front-end
- **Rust** engine: `git2` (diff), `tree-sitter` (symbols), `rusqlite` (cache)
- AI via the **`claude` CLI** (Claude Code) — Sonnet for structure, Haiku for labels

## Build from source

```sh
git clone https://github.com/JiwonKKang/loupe
cd loupe
npm install
npm run tauri dev        # run
npm run tauri build      # produce a universal .dmg / .app
```

Requires Node, Rust, and the Tauri prerequisites.

## Privacy

- Your token is stored only on your Mac (`~/Library/Application Support/com.jiwon.loupe/`),
  `0600`, and passed to the `claude` child process via env — never on a command line,
  never logged.
- The app itself opens no network connections; AI traffic is the `claude` CLI on your machine.

## Contributing

Issues and PRs welcome — language grammars, editor integrations, and UI polish especially.

## License

MIT © 강지원 (Jiwon Kang)
