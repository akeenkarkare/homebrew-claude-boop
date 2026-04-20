# claude-boop

Notifications for [Claude Code](https://docs.claude.com/en/docs/claude-code). Plays a sound when Claude needs your attention (permission prompt / idle) or finishes a response.

## Install

```sh
brew install akeenkarkare/claude-boop/claude-boop
claude-boop install
```

That's it — `install` patches `~/.claude/settings.json` to wire up `Notification` and `Stop` hooks. It preserves any existing hooks and is safe to run twice.

## Commands

| Command | What it does |
| --- | --- |
| `claude-boop play --event notification` | Play the permission/idle sound |
| `claude-boop play --event stop` | Play the "generation complete" sound |
| `claude-boop install` | Add hooks to `~/.claude/settings.json` |
| `claude-boop uninstall` | Remove claude-boop's hooks |

The `play` commands are what the hooks invoke — you usually don't run them by hand.

## Custom sounds

Sounds are compiled into the binary. To use your own, clone the repo, drop replacements into `assets/notification.aiff` and `assets/stop.aiff` (any format `afplay`/`paplay` can handle, renamed `.aiff`), and `cargo install --path .`.

## Platforms

- **macOS** — uses `afplay` (built in)
- **Linux** — uses `paplay`, `aplay`, or `ffplay` (whichever's installed)
- **Windows** — not yet supported

## Uninstall

```sh
claude-boop uninstall
brew uninstall claude-boop
```
