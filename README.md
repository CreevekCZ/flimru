# Flimru

Pick a device, run a Flutter launch configuration, and hot reload from your terminal. Your coding agent can use the same sessions.

![Flimru device picker with iPhone and Pixel simulators selected](docs/screenshots/device-picker.png)

## Install

**With an agent:** Copy this prompt into your coding assistant from your Flutter project. The private preview requires GitHub access.

```text
Set up Flimru in this Flutter checkout. On Apple Silicon, use gh to download
the v0.1.0 darwin-arm64 archive and SHA256SUMS from CreevekCZ/flimru. Verify
the checksum and install the binary on PATH. If the binary is unavailable,
clone that tag or the codex/release-v0.1.0 branch and install with Cargo.

Use this project's Flutter or FVM SDK, installing it if needed. Keep the
existing .vscode/launch.json, or create a Dart launch config if missing.
Run flimru --check. Register the absolute Flimru path as a local stdio MCP
server named flimru with argument mcp. Add a short project rule to check
existing Flimru sessions first and use Flimru to start and control runs.
Open flimru in a terminal here if possible and verify flimru session list.
Tell me if I need to restart this agent to load the tools.
```

**From source:** In the Flimru repo, run `cargo install --locked --path .`. Run `flimru` in your Flutter checkout.

![Flimru console with two live device tabs and a successful hot reload](docs/screenshots/live-console.png)

[Usage and shortcuts](docs/usage.md) · [MCP and CLI reference](docs/agents.md) · [MIT](LICENSE)
