# Flimru for agents

Start with the [setup prompt](../README.md#install), or register the local
stdio server yourself. Replace the path with the output of `command -v flimru`.

```sh
codex mcp add flimru -- /absolute/path/to/flimru mcp
claude mcp add --scope user --transport stdio flimru -- /absolute/path/to/flimru mcp
opencode mcp add flimru --global -- /absolute/path/to/flimru mcp
```

Run only your provider's command. The OpenCode command is for V2. In T3 Code,
use the selected local provider and restart the agent session after setup.

Open `flimru` in the Flutter checkout and start the agent in that same checkout
as the same user. If the agent starts elsewhere, pass
`mcp --project /absolute/path/to/flutter/project`. Verify with `flimru session list`
or `flimru_sessions` over MCP. An empty list is a working connection.

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
