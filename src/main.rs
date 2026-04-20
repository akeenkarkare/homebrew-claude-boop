use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const NOTIFICATION_SOUND: &[u8] = include_bytes!("../assets/notification.aiff");
const STOP_SOUND: &[u8] = include_bytes!("../assets/stop.aiff");
const SOUND_EXT: &str = "aiff";

#[derive(Parser)]
#[command(name = "claude-boop", version, about = "Cute notifications for Claude Code")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Play a sound for the given event (invoked by Claude Code hooks)
    Play {
        #[arg(long, value_enum)]
        event: Event,
    },
    /// Add claude-boop hooks to ~/.claude/settings.json
    Install,
    /// Remove claude-boop hooks from ~/.claude/settings.json
    Uninstall,
}

#[derive(Copy, Clone, ValueEnum)]
enum Event {
    Notification,
    Stop,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Play { event } => play(event),
        Cmd::Install => install(),
        Cmd::Uninstall => uninstall(),
    };
    if let Err(e) = result {
        eprintln!("claude-boop: {e}");
        std::process::exit(1);
    }
}

fn play(event: Event) -> Result<(), String> {
    let bytes = match event {
        Event::Notification => NOTIFICATION_SOUND,
        Event::Stop => STOP_SOUND,
    };
    let mut path = std::env::temp_dir();
    path.push(format!("claude-boop-{}.{}", std::process::id(), SOUND_EXT));
    {
        let mut f = fs::File::create(&path).map_err(|e| format!("temp file: {e}"))?;
        f.write_all(bytes).map_err(|e| format!("write: {e}"))?;
    }
    let status = player_command(&path);
    let _ = fs::remove_file(&path);
    status
}

#[cfg(target_os = "macos")]
fn player_command(path: &std::path::Path) -> Result<(), String> {
    Command::new("afplay")
        .arg(path)
        .status()
        .map_err(|e| format!("afplay: {e}"))?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn player_command(path: &std::path::Path) -> Result<(), String> {
    for cmd in ["paplay", "aplay", "ffplay"] {
        let args: &[&str] = if cmd == "ffplay" {
            &["-nodisp", "-autoexit", "-loglevel", "quiet"]
        } else {
            &[]
        };
        if let Ok(mut c) = Command::new(cmd).args(args).arg(path).spawn() {
            let _ = c.wait();
            return Ok(());
        }
    }
    Err("no audio player found (install pulseaudio, alsa-utils, or ffmpeg)".into())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn player_command(_path: &std::path::Path) -> Result<(), String> {
    Err("platform not supported yet".into())
}

fn settings_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("no home dir")?;
    Ok(home.join(".claude").join("settings.json"))
}

fn install() -> Result<(), String> {
    let path = settings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create settings dir: {e}"))?;
    }
    let mut root: Value = if path.exists() {
        let raw = fs::read_to_string(&path).map_err(|e| format!("read settings: {e}"))?;
        if raw.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&raw).map_err(|e| format!("parse settings.json: {e}"))?
        }
    } else {
        json!({})
    };

    let hooks = root
        .as_object_mut()
        .ok_or("settings.json root is not an object")?
        .entry("hooks")
        .or_insert_with(|| json!({}));

    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.canonicalize().ok())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "claude-boop".to_string());

    let notif_cmd = format!("{exe} play --event notification");
    let stop_cmd = format!("{exe} play --event stop");
    add_hook(hooks, "Notification", &notif_cmd)?;
    add_hook(hooks, "Stop", &stop_cmd)?;

    let pretty = serde_json::to_string_pretty(&root).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, pretty + "\n").map_err(|e| format!("write settings: {e}"))?;
    println!("claude-boop: installed hooks in {}", path.display());
    println!("claude-boop: using binary at {exe}");
    Ok(())
}

fn uninstall() -> Result<(), String> {
    let path = settings_path()?;
    if !path.exists() {
        println!("claude-boop: no settings.json at {}", path.display());
        return Ok(());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("read settings: {e}"))?;
    let mut root: Value =
        serde_json::from_str(&raw).map_err(|e| format!("parse settings.json: {e}"))?;
    if let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        for event in ["Notification", "Stop"] {
            remove_hook(hooks, event);
        }
    }
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, pretty + "\n").map_err(|e| format!("write settings: {e}"))?;
    println!("claude-boop: removed hooks from {}", path.display());
    Ok(())
}

fn add_hook(hooks: &mut Value, event: &str, command: &str) -> Result<(), String> {
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or("settings.hooks is not an object")?;
    let matchers = hooks_obj
        .entry(event.to_string())
        .or_insert_with(|| json!([]));
    let arr = matchers
        .as_array_mut()
        .ok_or_else(|| format!("settings.hooks.{event} is not an array"))?;

    let already_present = arr.iter().any(|matcher| {
        matcher
            .get("hooks")
            .and_then(|h| h.as_array())
            .map(|inner| {
                inner
                    .iter()
                    .any(|h| h.get("command").and_then(|c| c.as_str()) == Some(command))
            })
            .unwrap_or(false)
    });
    if already_present {
        return Ok(());
    }

    arr.push(json!({
        "hooks": [{ "type": "command", "command": command }]
    }));
    Ok(())
}

fn is_claude_boop_command(cmd: &str) -> bool {
    let trimmed = cmd.trim_start();
    if trimmed.starts_with("claude-boop ") {
        return true;
    }
    let first = trimmed.split_whitespace().next().unwrap_or("");
    std::path::Path::new(first)
        .file_name()
        .and_then(|f| f.to_str())
        .map(|f| f == "claude-boop")
        .unwrap_or(false)
}

fn remove_hook(hooks: &mut serde_json::Map<String, Value>, event: &str) {
    let Some(matchers) = hooks.get_mut(event).and_then(|v| v.as_array_mut()) else {
        return;
    };
    matchers.retain(|matcher| {
        let Some(inner) = matcher.get("hooks").and_then(|h| h.as_array()) else {
            return true;
        };
        !inner.iter().any(|h| {
            h.get("command")
                .and_then(|c| c.as_str())
                .map(is_claude_boop_command)
                .unwrap_or(false)
        })
    });
    if matchers.is_empty() {
        hooks.remove(event);
    }
}
