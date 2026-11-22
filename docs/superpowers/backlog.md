# tomakast: feature backlog

Features from the user's original request (2026-11-18) that are outside the first spec. Each one gets its own spec (brainstorm → spec → plan). The user's wishes are recorded as stated; open questions are what the user left undecided.

Spec brainstorming for any item can start at any time, since it touches no code. The "Implementation can start" column says when building it in parallel with the main line stops colliding with unfinished foundation work.

| Feature | User's wishes | Open questions | Loader | Implementation can start |
|---|---|---|---|---|
| Dictation | Like Raycast Dictation. Local or remote speech models, then post-processing that depends on the target app | Which models (local whisper-class vs API); how the target app is detected on Wayland; how text is inserted into the focused app on Wayland; what post-processing per app means | Built-in (mic, focused app, text injection) | After plan 2b-1 Task 2 (async module trait; own overlay window). The macOS part after plan 2a |
| Translator | Through an API like Google Translate | Which provider(s); API key handling | WASM plugin (`http`) | After plan 2b-2 (WIT 0.1 frozen, `http`, list/detail views) |
| Converter | Folded into calc (2026-11-22): colours, number bases, more units and city short names, by `specs/2026-11-22-calc-colors-bases-units-design.md` | — | Calc (`plugins/calc`) | Done in calc |
| Text transforms | base64, URL encoding, letter case and similar text conversions | Which transforms; one plugin or several | WASM plugin(s) | Any time |
| Caffeinate | Keep the machine awake | Timer modes; indicator | Built-in (power inhibit APIs) | After plan 2b-1 Task 2 (async module trait) |
| Clipboard history | History of the clipboard | Retention, sensitive-content filtering (password managers), images; on Wayland needs a data-control protocol | Built-in (clipboard watching) | After plan 2b-1 views (list + detail), Linux part after plan 2b-1 Task 2 |
| Window management | Manage windows | Which operations (snap, move between monitors, layouts); on KDE via KWin scripting/D-Bus; macOS via Accessibility | Built-in | After plan 2b-1 Task 2 (Linux); after plan 2a (macOS) |
| Notes | "Notes???": unsure whether they are needed at all | Whether to do it; scope vs a real notes app | Built-in (own editor window) or WASM | Decide first |
| Sync | Cloud or any other sync, "need to think" | Mechanism (own server, user's cloud drive, CRDT, git…); what syncs (usage history, settings, plugin data, notes, calc sheets) | Core | After plan 2b-2 (storage namespaces settled) |
| File search | Search apps, commands and files with smart ranking | Index source (own indexer vs Baloo/Spotlight/Windows Search); scale of hundreds of thousands of paths | Built-in dynamic source | After plan 2b-1 Task 2 (dynamic sources) |
| Plugin manager | Comes later as its own feature/module | Distribution (registry vs git URLs), updates, signing | Core + UI | After plan 2b-2 |
| Leader key hint panel | Which-key style: after the leader is pressed and ~0.4 s pass with no key, a small panel lists the next possible keys and what each does, nested groups marked with `›`; people who know the keys never see it. "A cool idea, it must be remembered" (2026-11-22) | Delay (Neovim which-key 0.2 s, Emacs 1 s, Leader Key.app 2 s plus `?` on demand); layout and placement; whether `?` opens it at once | UI, on top of leader keys (a user-chosen leader, then a key, as a namespace for hotkeys free of system and app conflicts) | After the leader-key feature lands |
