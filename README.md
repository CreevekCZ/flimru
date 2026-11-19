# Flimru

Run Flutter from your terminal. Pick a device and a VS Code launch configuration, then manage logs and hot reload in the same window. Local agents can share those sessions.

## Quick start

Install the [Apple Silicon preview](https://github.com/CreevekCZ/flimru/releases) with GitHub access, or run `cargo install --path . --locked` from a source checkout. See [installation instructions](docs/usage.md#install) for both.

In your Flutter project, add a [launch configuration](docs/usage.md#run-flutter) and make sure Flutter or your project's FVM SDK is available. Then run:

```sh
flimru --check
flimru
```

Pick a device and configuration with **Enter**. Open **Actions** with **Ctrl+p**, or quit with **Ctrl+q**, then **Enter**.

[Usage and shortcuts](docs/usage.md) · [Agent setup and CLI](docs/agents.md) · [MIT license](LICENSE)
