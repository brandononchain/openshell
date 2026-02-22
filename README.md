# OpenShell

OpenShell is a cross-platform Tauri desktop app for running OpenClaw agents locally with Docker.

## Core features

- Mission Control dashboard for agent lifecycle operations
- Template-first agent wizard (`openclaw-default`, `openclaw-ui-bridge`)
- SQLite-backed agent registry with runtime sync fields
- Docker diagnostics (binary, daemon, compose flavor + fix hints)
- Logs streaming (`stdout` + `stderr`) and embedded xterm terminal
- Safe ops allowlist + container shell attach (container-only, no host shell)
- Export/import agent bundles (`.zip`)
- Observability-lite via `docker stats --no-stream`
- Auto-update support with Tauri updater

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

## Build distributables

```bash
pnpm tauri build
```

## Release automation

- Workflow: `.github/workflows/release.yml`
- Triggers:
  - tag push `v*`
  - manual `workflow_dispatch`
- Matrix builds for Linux/macOS/Windows
- Bundles are uploaded to GitHub Releases on tag builds

## Updater notes

- Updater configured in `apps/desktop/src-tauri/tauri.conf.json`
- Endpoint points to GitHub release feed JSON (`latest.json`)
- Settings panel includes:
  - current app version
  - check for updates
  - download and install update

### Example update feed (`latest.json`)

```json
{
  "version": "0.1.1",
  "notes": "Bug fixes and stability improvements",
  "pub_date": "2026-01-01T00:00:00Z",
  "platforms": {
    "darwin-aarch64": {
      "signature": "...",
      "url": "https://github.com/openclaw/openshell/releases/download/v0.1.1/OpenShell.app.tar.gz"
    }
  }
}
```
