# Use Flimru with an agent

## Register the MCP server

Install Flimru and run `command -v flimru` to find its absolute path. Use the
same executable for the terminal launcher and your agent. Run the command for
your local agent provider, replacing `/absolute/path/to/flimru`.

```sh
# Codex
codex mcp add flimru -- /absolute/path/to/flimru mcp

# Claude Code
claude mcp add --scope user --transport stdio flimru -- /absolute/path/to/flimru mcp

# OpenCode V2
opencode mcp add flimru --global -- /absolute/path/to/flimru mcp
```

For other MCP clients, configure a local stdio server with that executable and
`["mcp"]` as its argument list. In T3 Code, configure the selected local agent
provider, then start a new agent session or use **Restart agent session**.
[Codex](https://developers.openai.com/codex/mcp) · [Claude Code](https://code.claude.com/docs/en/mcp) · [OpenCode](https://opencode.ai/docs/mcp-servers/)

## Connect to a Flutter checkout

1. Open `flimru` in a terminal in the Flutter project and leave the window open.
2. Open a fresh local agent session in that same checkout or worktree, on the same
   machine and under the same user account.
3. Ask the agent to list Flimru sessions. An empty list means the connection
   works and there are no runs. If it can't connect, run `flimru session list`
   from the checkout to check the launcher independently of the MCP client.

The MCP server locates the nearest parent with `.vscode/launch.json`, including
its own working directory. It uses its working directory if it finds no file.
If the client starts in a different directory, pass
`mcp --project /absolute/path/to/flutter/project` as its arguments. Each
checkout has its own sessions. The first Flimru window in that checkout owns
the shared sessions, which keep running if the agent disconnects.

## Give this instruction to the agent

Copy into the Flutter project's `AGENTS.md` or `CLAUDE.md`.

> Use Flimru for Flutter app runs. First check `flimru_sessions` and reuse a
> matching run. Use the exact device ID and configuration name from
> `flimru_targets` with `flimru_start`. If names repeat, pass the one-based
> `config_index` instead of the name. Keep the returned session ID for
> `flimru_logs` and `flimru_control`. Use the device tool for viewing and
> interacting with the app, and Flimru to launch, reload, restart, or stop it.
> If MCP is unavailable, use `flimru session` in this checkout. If Flimru is
> closed, ask me to open it here.

Use `next_config_offset` as the next `config_offset` when targets are paged.
For logs, omit `cursor` to read recent output or use `0` to start at the
earliest retained output. Pass the returned `cursor` on each subsequent read.
Check `truncated`, `state`, and `outcome`. Reload and restart acknowledge
delivery. Read the subsequent Flutter logs for the result.

## CLI fallback

Run these commands from the Flutter checkout while Flimru is open.

```sh
flimru session list
flimru session targets
flimru session start --device DEVICE_ID --config 'Debug launch name'
flimru session inspect SESSION_ID
flimru session logs SESSION_ID --follow
flimru session reload SESSION_ID
flimru session restart SESSION_ID
flimru session stop SESSION_ID
```

Use `--config-index NUMBER` when the launch name is ambiguous. Use
`flimru session --project /absolute/path/to/project list` to select a checkout
explicitly. **Ctrl+c** ends log following and leaves the run active.
