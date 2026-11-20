# Flimru for agents

## One-time setup

On macOS or Linux, use the [setup prompt](../README.md#install), or run your
client's command below. Replace the path with the output of `command -v flimru`.

```sh
codex mcp add flimru -- /absolute/path/to/flimru mcp
claude mcp add --scope user --transport stdio flimru -- /absolute/path/to/flimru mcp
opencode mcp add flimru --global -- /absolute/path/to/flimru mcp
```

Save the Flutter rule from the setup prompt in your client's global instructions.

| Client | Default global instructions |
| --- | --- |
| Codex | [`~/.codex/AGENTS.md`](https://developers.openai.com/codex/guides/agents-md) |
| Claude Code | [`~/.claude/CLAUDE.md`](https://code.claude.com/docs/en/memory) |
| OpenCode V2 | [`~/.config/opencode/AGENTS.md`](https://opencode.ai/docs/instructions) |

In T3 Code, use the settings for your selected local provider. Start a fresh
agent session after registering the server.

## Run from any Flutter checkout

Open `flimru` and start your agent in the same checkout under your user account.
Flimru MCP uses the folder it starts in to locate the checkout. Verify with
`flimru_sessions`. An empty list means the connection works.
If your client starts MCP elsewhere, use the CLI from the checkout or pass
`flimru session --project /path/to/flutter/project list`.

| Task | MCP | Shell |
| --- | --- | --- |
| Find existing runs | `flimru_sessions` | `flimru session list` |
| Find devices and configs | `flimru_targets` | `flimru session targets` |
| Start | `flimru_start` | `flimru session start --device DEVICE_ID --config 'Launch name'` |
| Follow output | `flimru_logs` | `flimru session logs SESSION_ID --follow` |
| Control one run | `flimru_control` | `flimru session reload SESSION_ID`, `restart`, or `stop` |

Reuse a matching session. Pass the exact device ID and launch name, or use the
one-based `config_index` (`--config-index` in the CLI) for duplicate names.
Save the session ID and pass the returned `cursor` on the next log read.
Check logs for Flutter's reload or restart result.
