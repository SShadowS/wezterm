# Tmux Compat Debugging Guide

## Enabling Logs

Set the `WEZTERM_LOG` environment variable before launching WezTerm:

```bash
# All debug-level logs
WEZTERM_LOG=debug

# Targeted: only tmux compat related modules
WEZTERM_LOG=mux::domain=debug,wezterm_gui=debug,mux::tmux_compat_server=debug
```

## Log Messages

| Scenario | Level | Message |
|---|---|---|
| Feature enabled, startup | `info` | "tmux compat mode is enabled" |
| Feature disabled | `debug` | "tmux compat mode is disabled" |
| CC server started | `info` | "tmux CC compat listener started on ..." |
| CC server failed | `warn` | "Failed to start tmux CC compat server: ..." |
| Env vars set on shell spawn | `debug` | "tmux compat: setting TMUX=..." |
| Shim dir prepended to PATH | `debug` | "tmux compat: prepended ... to PATH" |
| Shim dir missing | `warn` | "tmux compat: shim directory ... does not exist..." |
| CC socket env var missing | `warn` | "tmux compat enabled but WEZTERM_TMUX_CC not set..." |
| Connection accepted | `trace` | "tmux CC: accepted new connection" |
| Connection error | `error` | "tmux CC connection error: ..." |
