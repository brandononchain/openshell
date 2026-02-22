# OpenShell

OpenShell is a cross-platform Tauri desktop app for running OpenClaw agents locally with Docker. It provides:

- Mission Control dashboard for agent lifecycle operations
- Agent template + env registry persisted in SQLite
- Live logs streaming from Docker Compose
- Embedded xterm.js ops console with strict allowlist commands
- Local app-data managed agent workdirs and compose/env generation

## Tech Stack

- **Desktop shell**: Tauri v2 (Rust backend)
- **UI**: React + TypeScript + Vite + Tailwind + shadcn-style primitives
- **Terminal UI**: xterm.js
- **Data**: SQLite + sqlx migrations
- **Runtime orchestration**: Docker + Docker Compose (`docker compose` fallback `docker-compose`)

## Prerequisites

1. Node.js 20+
2. pnpm 9+
3. Rust stable toolchain
4. Docker Desktop (or Docker Engine + Compose plugin)

## Development

```bash
pnpm install
pnpm dev
```

`pnpm dev` launches the Vite frontend for the Tauri desktop app.

## Build distributables

```bash
pnpm tauri build
```

Produces OS-specific distributables for Windows/macOS/Linux (depending on host toolchains).

## Project structure

- `apps/desktop/src`: React frontend (Mission Control, wizard, detail tabs)
- `apps/desktop/src-tauri/src/lib.rs`: Tauri command surface
- `apps/desktop/src-tauri/src/supervisor.rs`: Docker supervisor, SQLite registry, logs streamer, ops allowlist
- `apps/desktop/src-tauri/migrations`: SQLite schema

## MVP capabilities

- Docker installation and compose command detection
- Agent CRUD-lite: create + list
- Agent lifecycle: start/stop/restart via Compose
- Logs follow streaming via Tauri events (`agent-log-line`)
- Ops command allowlist (`docker ps`, compose up/down/logs, show config, open folder)
- Local persistence in SQLite and per-agent filesystem directory
