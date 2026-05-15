# claude-boop

<p align="center">
  <img src="assets/icon.svg" width="104" alt="claude-boop icon">
</p>

Lightweight cues for [Claude Code](https://docs.claude.com/en/docs/claude-code). It plays a sound, updates your terminal title, and shows a native desktop notification when Claude needs attention or finishes a response.

This is an unofficial Claude Code helper. The icon uses the Claude AI symbol made available as CC0 on Wikimedia Commons, with a small notification badge added for claude-boop.

## Install

**macOS / Linux (Homebrew):**

```sh
brew install akeenkarkare/claude-boop/claude-boop
claude-boop install
```

**Windows:** download `claude-boop-vX.Y.Z-x86_64-pc-windows-msvc.zip` (or `aarch64-pc-windows-msvc` on ARM) from the [latest release](https://github.com/akeenkarkare/homebrew-claude-boop/releases/latest), extract `claude-boop.exe` somewhere on your `PATH`, then:

```powershell
claude-boop install
```

That's it — `install` patches `~/.claude/settings.json` (`%USERPROFILE%\.claude\settings.json` on Windows) to wire up `Notification` and `Stop` hooks. It preserves any existing hooks and is safe to run twice.

By default:

- Sound is enabled
- Terminal title updates are enabled
- Native notifications are enabled when the platform has a supported backend

## Commands

| Command | What it does |
| --- | --- |
| `claude-boop install` | Add hooks to `~/.claude/settings.json` and create default config |
| `claude-boop install --visual sound,title,notify` | Install hooks and explicitly enable selected cue channels |
| `claude-boop uninstall` | Remove claude-boop's hooks |
| `claude-boop play --event notification` | Run configured cues for the permission/idle event |
| `claude-boop play --event stop` | Run configured cues for the "generation complete" event |
| `claude-boop play --event notification --sound-only` | Play only the notification sound |
| `claude-boop play --event stop --no-sound --title --notify` | Override cue channels for one invocation |
| `claude-boop config show` | Print current config |
| `claude-boop config set sound true` | Enable or disable sound |
| `claude-boop config set title true` | Enable or disable terminal title updates |
| `claude-boop config set notify false` | Enable or disable native notifications |
| `claude-boop config set quiet "22:00-08:00"` | Set quiet hours (sound + notifications muted) |
| `claude-boop config set quiet ""` | Disable quiet hours |
| `claude-boop config reset` | Reset config to defaults |
| `claude-boop doctor` | Check hooks, config, and local cue backends |

The `play` commands are what the hooks invoke — you usually don't run them by hand.

## Testing a local build

If you are testing a branch before a release, install that local binary and then reinstall the hooks with it:

```sh
cargo install --path . --force
~/.cargo/bin/claude-boop uninstall
~/.cargo/bin/claude-boop install
~/.cargo/bin/claude-boop doctor
```

Check that `~/.claude/settings.json` points at the local binary:

```sh
grep claude-boop ~/.claude/settings.json
```

You should see only one `Notification` command and one `Stop` command. If you previously tested with Homebrew, `cargo run`, and `cargo install`, running `uninstall` with the current binary removes the older claude-boop hook entries before reinstalling the new ones.

You can test the cues without waiting for Claude Code:

```sh
~/.cargo/bin/claude-boop play --event notification
~/.cargo/bin/claude-boop play --event stop
```

For the Claude Code VS Code extension, run `install`, restart VS Code, and trigger a permission prompt or a completed response. Sound and native notifications should behave the same as the CLI because the extension shares `~/.claude/settings.json`; terminal title updates may depend on how the extension hosts the hook process.

## Configuration

Config lives at `~/.claude/claude-boop.json` (`%USERPROFILE%\.claude\claude-boop.json` on Windows):

```json
{
  "sound": true,
  "title": true,
  "notify": true,
  "quiet": "22:00-08:00",
  "dangerPatterns": [
    "(?:^|[\\s;&|])rm\\s+(?:-[a-zA-Z]*[rRfFd]|--recursive|--force)",
    "(?:^|[\\s;&|])git\\s+push\\s+(?:--force\\b|-f\\b|--force-with-lease\\b)"
  ],
  "titles": {
    "notification": "Claude • Waiting for approval",
    "stop": "Claude • Done ✓"
  },
  "messages": {
    "notification": "Claude needs your attention",
    "stop": "Claude finished"
  }
}
```

The easiest way to change channels is through the CLI:

```sh
claude-boop config set sound true
claude-boop config set title true
claude-boop config set notify false
claude-boop config set quiet "22:00-08:00"
```

You can edit the JSON directly if you want custom terminal titles, notification messages, or risk patterns.

### Quiet hours

When the current local time falls inside the configured window, claude-boop suppresses **sound** and **native notifications** but still updates the terminal title (silent visual cue). The window is `HH:MM-HH:MM` in 24-hour local time and supports wrap-around midnight (`22:00-08:00` means 10pm through 8am the next morning). Set `quiet` to an empty string to disable.

### Danger sound for risky Bash commands

claude-boop installs a `PreToolUse` hook on `Bash` that plays a distinct, more alarming sound when Claude is about to run something risky — before you decide whether to allow it. Defaults match:

- `rm -rf` / `rm -r` / `rm -f` and `--recursive` / `--force` long forms
- `git push --force` / `-f` / `--force-with-lease`
- `git reset --hard`, `git clean -f`
- `sudo`
- `curl … | sh` / `wget … | sh` (pipe-to-shell)
- writes to `/dev/sd*` or `/dev/hd*`
- `dd if=`, `mkfs.*`, `chmod -R 777`-style, fork bombs
- `drop database` / `drop table`

To add your own, edit the `dangerPatterns` array (Rust `regex` syntax, case-insensitive). Empty the array to silence the danger sound entirely.

## Custom sounds

Sounds are compiled into the binary. To use your own, clone the repo, drop replacements into `assets/notification.{aiff,wav}`, `assets/stop.{aiff,wav}`, and `assets/danger.{aiff,wav}` (the `.aiff` files are used on macOS/Linux, the `.wav` files on Windows), then `cargo install --path .`.

## Platforms

- **macOS** — uses `afplay` for sound and `osascript` for notifications
- **Linux** — uses `paplay`, `aplay`, or `ffplay` for sound, and `notify-send` for notifications when available
- **Windows** — uses PowerShell's built-in `System.Media.SoundPlayer`

Terminal titles use a standard ANSI escape sequence and are written to stderr so normal hook output stays quiet.

Unsupported cue backends are skipped silently during hook runs. Use `claude-boop doctor` when you want to see what is available on your machine.

## Troubleshooting

Run:

```sh
claude-boop doctor
```

It checks whether `settings.json` exists, whether both Claude hooks are installed, whether the config file is valid, whether a sound backend is available, and whether a native notification backend is available.

If sounds do not play on Linux, install one of `pulseaudio`, `alsa-utils`, or `ffmpeg`. If desktop notifications do not appear on Linux, install `libnotify` or a package that provides `notify-send`.

If you want to return to defaults:

```sh
claude-boop config reset
claude-boop install
```

## Uninstall

```sh
claude-boop uninstall
brew uninstall claude-boop
```
