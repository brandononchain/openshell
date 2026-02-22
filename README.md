# OpenShell

OpenShell is a cross-platform Tauri desktop app for running OpenClaw agents locally with Docker. It provides:

- Mission Control dashboard for agent lifecycle operations
- Agent template + env registry persisted in SQLite
- Live logs streaming from Docker Compose (stdout + stderr)
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

## MVP capabilities

- Docker checks: installation, daemon running/permissions (`docker info`), compose availability, fix hints.
- Agent management: create, import folder, duplicate, delete (optional `down -v`).
- Agent lifecycle: start/stop/restart via Compose with explicit project names.
- Authoritative status sync from labeled containers (`openshell.agent_id`, `openshell.agent_name`).
- Logs follow streaming via Tauri events, including stream metadata (`stdout`/`stderr`).
- Ops command allowlist (`docker ps`, compose up/down/logs, show config, open folder).
- Local persistence in SQLite and per-agent filesystem directory.
