# CLI Reference

## Telegram commands (General topic)

| Command | Description |
|---------|-------------|
| `/status` | Show fleet status, context %, and costs |
| `/restart` | In-process restart all instances (no process exit) |
| `/upgrade` | Exit process to apply new code (requires launchd/systemd auto-restart) |
| `/sysinfo` | Show detailed system diagnostics (version, load, IPC status) |

All other operations (create/delete/start instances, delegate tasks) are handled by the General instance through natural language.

## Service management

```bash
agend start                     # Start AgEnD service (requires install)
agend stop                      # Stop AgEnD service
agend restart                   # Restart AgEnD service
agend update                    # Update AgEnD to latest version and restart
agend reload                    # Hot-reload config (re-read fleet.yaml, start new instances)
```

## Fleet management

```bash
agend fleet start               # Start all instances (manual mode)
agend fleet stop                # Stop all instances
agend fleet restart             # Graceful restart (wait for idle, same code)
agend fleet restart <name>      # Restart a specific instance
agend fleet restart --reload    # Restart with new code (suicide + system restart)
agend fleet status              # Show instance status overview
agend fleet logs <name>         # Show instance logs
agend fleet history             # Show event history (cost, rotations, hangs)
agend fleet activity            # Show activity log (collaboration, tool calls, messages)
agend fleet activity --format mermaid  # Output activity as Mermaid sequence diagram
agend fleet cleanup             # Remove orphaned instance directories
agend fleet cleanup --dry-run   # Preview cleanup without deleting
```

## Instance tools

```bash
agend ls                        # List instances with JSON output
agend attach [name]             # Attach to instance tmux window (fuzzy match, interactive menu)
agend logs [name]               # Show instance output (ANSI stripped), -n/--lines, -f/--follow
agend export-chat               # Export fleet activity as HTML chat log
agend export-chat --from <date> --to <date> -o <path>
```

## Backend diagnostics

```bash
agend backend doctor [backend]  # Check backend environment (binary, auth, tmux, TERM)
agend backend trust <backend>   # Pre-trust working directories (avoid Gemini CLI trust dialogs)
```

## Web Dashboard

```bash
agend web                       # Open Web UI dashboard in browser
```

## Schedules

```bash
agend schedule list             # List all schedules
agend schedule add              # Add a schedule from CLI
agend schedule delete <id>      # Delete a schedule
agend schedule enable <id>      # Enable a schedule
agend schedule disable <id>     # Disable a schedule
agend schedule history <id>     # Show schedule run history
agend schedule trigger <id>     # Manually trigger a schedule
agend schedule update <id>      # Update schedule parameters
```

## Topic bindings

```bash
agend topic list                # List topic bindings
agend topic bind <name> <tid>   # Bind instance to topic
agend topic unbind <name>       # Unbind instance from topic
```

## Access control

```bash
agend access list <name>        # List allowed users
agend access add <name> <uid>   # Add allowed user
agend access remove <name> <uid> # Remove user
agend access lock <name>        # Lock instance access (whitelist only)
agend access unlock <name>      # Unlock instance access (enable pairing)
agend access pair <name> <uid>  # Generate pairing code
```

## Setup & installation

```bash
agend quickstart                # Simplified 4-question setup (recommended for new users)
agend init                      # Full interactive setup wizard (9 steps)
agend install                   # Install as system service (launchd/systemd)
agend install --activate        # Install and start immediately
agend uninstall                 # Remove system service
agend export [path]             # Export config for device migration
agend export --full [path]      # Export config + all instance data
agend import <file>             # Import config from export file
```
