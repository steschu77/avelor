# Avelor
![CI](https://github.com/steschu77/avelor/actions/workflows/ci.yml/badge.svg)

Avelor is a privacy-first desktop application for organizing the paperwork of everyday life. It brings documents, emails, projects, conversations, tasks, and calendar events together in one place, built around a simple idea: **your personal data should remain yours.**

Avelor targets both Windows and Linux.

## Why Avelor?

This project started with a personal need: getting my own paperwork organized.

Documents accumulate in folders, important information gets buried in emails, and keeping track of deadlines, appointments, and ongoing matters becomes a task in itself. I wanted one place to bring these things together, connect related information, and make it easier to find what matters — without handing my personal archive over to another cloud service.

Avelor is my attempt to build that tool. It is developed for practical everyday use, with a focus on what one person can reasonably manage: organizing information, keeping track of ongoing matters, and getting help from AI when it is useful.

It is a personal project, not a commercial product or a claim to have solved personal information management. The goal is to make private-life paperwork more manageable, one useful feature at a time.

## Core Principles

- **Your files are the source of truth.** A vault is a user-owned folder of plain Markdown and YAML files, not a proprietary database. You can inspect, back up, edit, and move your data with ordinary tools.
- **Privacy first.** Personal information stays in local files under your control. AI features can use local models, so sensitive documents can be processed without sending their contents to a cloud AI provider. External integrations, such as email and calendar synchronization, remain available when you choose to use them.
- **Edits stay with the document.** Changes are saved directly to the source file; the application does not maintain drafts or temporary document copies.
- **Connect the pieces.** Documents, emails, projects, threads, conversations, tasks, and calendar events can be organized together rather than treated as isolated information in separate applications.
- **AI should assist, not take over.** AI can help process paperwork and make information more useful, while the user remains in control of the data and the workflow.
- **Stay useful within realistic limits.** Avelor aims to cover the recurring needs of everyday paperwork without becoming an unnecessarily complex, all-purpose productivity suite.
- **Stay focused and lightweight.** The application avoids an npm or JavaScript bundler dependency and keeps its underlying documents accessible outside the application.
- **AI-friendly by design.** Markdown and YAML are human-readable, structured, and compact. This makes it cheaper for AI features to understand your data, especially when running local models.

## How It Compares

Avelor builds on ideas found in several existing tools for managing personal information. Applications such as [Obsidian](https://obsidian.md/) and [Joplin](https://joplinapp.org/) demonstrate the value of local, file-based notes; [Paperless-ngx](https://docs.paperless-ngx.com/), [Papra](https://papra.app/), and [Docspell](https://docspell.org/) focus on document archiving and retrieval; and [Nextcloud](https://nextcloud.com/) brings together services such as files, mail, and calendars in a self-hosted environment.

| Alternative | What it does well | Where Avelor differs |
|---|---|---|
| **Obsidian** | Local Markdown notes, linked knowledge, and extensibility | Provides a dedicated model for personal paperwork and its relationships to correspondence, projects, tasks, and events |
| **Joplin** | Notes, to-dos, offline use, and optional encrypted synchronization | Organizes a broader set of personal information around documents and their related activities |
| **Paperless-ngx, Papra, Docspell** | Document archiving, indexing, retrieval, and metadata management | Connects paperwork to the ongoing matters of everyday life, rather than focusing primarily on the document archive |
| **Nextcloud** | An integrated, self-hosted ecosystem for files, mail, calendars, and other services | Uses a local desktop application and a user-owned folder of Markdown and YAML files as its foundation |

## Building

On Linux GTK 4.10 or later with its development files (`libgtk-4-dev` and `libwebkitgtk-6.0-dev` on Ubuntu
24.04 and later):

```sh
sudo apt install libgtk-4-dev libwebkitgtk-6.0-dev
```

```sh
cargo run
```

To open the sample vault in `data/forged` instead of the last one (edits
change its files; `git restore data/forged` undoes them):

```sh
cargo run -- data/forged
```

Signing in to Google needs the app's OAuth client (a Desktop client of a
Google Cloud project) in the build's environment: `AVELOR_GOOGLE_CLIENT_ID` and
`AVELOR_GOOGLE_CLIENT_SECRET`. A build without them fails.

On Linux the desktop shows the app and its icon once they are installed:

```sh
cargo install --path app
cp -r app/res/icons ~/.local/share/
install -Dm644 -t ~/.local/share/applications app/res/io.github.steschu77.Avelor.desktop
```

The checks every change passes:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Documentation

[`docs/architecture.md`](docs/architecture.md) is the entry point: what the app is, the rules it is built on, and [the map of the other documents](docs/architecture.md#documents).

## Development and AI Assistance

Avelor has been developed with the assistance of [Claude.ai](https://claude.ai/).

AI assistance is a tool in the development process, not a substitute for code ownership. Every commit has been carefully reviewed as if I had written the code myself. Architectural decisions, implementation choices, and the responsibility for the resulting software remain mine.

## License

This project is licensed under the [MIT license](LICENSE).

The icons in `app/static/icons` are from
[Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons)
by Microsoft, under the MIT license
([`app/static/icons/LICENSE`](app/static/icons/LICENSE)).
