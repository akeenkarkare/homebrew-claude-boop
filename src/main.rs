use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[cfg(target_os = "windows")]
const NOTIFICATION_SOUND: &[u8] = include_bytes!("../assets/notification.wav");
#[cfg(target_os = "windows")]
const STOP_SOUND: &[u8] = include_bytes!("../assets/stop.wav");
#[cfg(target_os = "windows")]
const DANGER_SOUND: &[u8] = include_bytes!("../assets/danger.wav");
#[cfg(target_os = "windows")]
const SOUND_EXT: &str = "wav";

#[cfg(not(target_os = "windows"))]
const NOTIFICATION_SOUND: &[u8] = include_bytes!("../assets/notification.aiff");
#[cfg(not(target_os = "windows"))]
const STOP_SOUND: &[u8] = include_bytes!("../assets/stop.aiff");
#[cfg(not(target_os = "windows"))]
const DANGER_SOUND: &[u8] = include_bytes!("../assets/danger.aiff");
#[cfg(not(target_os = "windows"))]
const SOUND_EXT: &str = "aiff";

const DEFAULT_DANGER_PATTERNS: &[&str] = &[
    r"(?:^|[\s;&|])rm\s+(?:-[a-zA-Z]*[rRfFd]|--recursive|--force)",
    r"(?:^|[\s;&|])git\s+push\s+(?:--force\b|-f\b|--force-with-lease\b)",
    r"(?:^|[\s;&|])git\s+reset\s+--hard\b",
    r"(?:^|[\s;&|])git\s+clean\s+(?:-[a-zA-Z]*f|--force)",
    r"(?:^|[\s;&|])sudo\b",
    r"(?:curl|wget)\b[^|&;]*\|\s*(?:sh|bash|zsh|ksh)\b",
    r">\s*/dev/[sh]d[a-z]",
    r"(?:^|[\s;&|])dd\s+if=",
    r":\(\)\s*\{\s*:\|:&\s*\}",
    r"(?:^|[\s;&|])mkfs\.",
    r"(?:^|[\s;&|])chmod\s+-R\s+[0-7]*7[0-7]*7",
    r"\bdrop\s+(?:database|table)\b",
];

#[derive(Parser)]
#[command(
    name = "claude-boop",
    version,
    about = "Cute notifications for Claude Code"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Play cues for the given event (invoked by Claude Code hooks)
    Play {
        #[arg(long, value_enum)]
        event: Event,
        /// Play only the sound channel, ignoring title and notification config
        #[arg(long)]
        sound_only: bool,
        /// Disable sound for this invocation
        #[arg(long)]
        no_sound: bool,
        /// Enable terminal title for this invocation
        #[arg(long)]
        title: bool,
        /// Enable native notification for this invocation
        #[arg(long)]
        notify: bool,
    },
    /// Inspect a PreToolUse hook payload from stdin and play danger sound if risky
    Pretooluse,
    /// Add claude-boop hooks to ~/.claude/settings.json
    Install {
        /// Explicit channels to enable: sound,title,notify
        #[arg(long)]
        visual: Option<String>,
    },
    /// Remove claude-boop hooks from ~/.claude/settings.json
    Uninstall,
    /// Show or update ~/.claude/claude-boop.json
    Config {
        #[command(subcommand)]
        command: ConfigCmd,
    },
    /// Check Claude settings, config, and local cue backends
    Doctor,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print the current config
    Show,
    /// Set sound/title/notify (true|false), or quiet ("HH:MM-HH:MM" or "")
    Set { key: String, value: String },
    /// Reset config to defaults
    Reset,
}

#[derive(Copy, Clone, ValueEnum, Debug, Eq, PartialEq)]
enum Event {
    Notification,
    Stop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BoopConfig {
    sound: bool,
    title: bool,
    notify: bool,
    quiet: Option<QuietWindow>,
    danger_patterns: Vec<String>,
    notification_title: String,
    stop_title: String,
    notification_message: String,
    stop_message: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct QuietWindow {
    start_minutes: u16,
    end_minutes: u16,
}

impl QuietWindow {
    fn contains(&self, minute_of_day: u16) -> bool {
        if self.start_minutes == self.end_minutes {
            return false;
        }
        if self.start_minutes < self.end_minutes {
            minute_of_day >= self.start_minutes && minute_of_day < self.end_minutes
        } else {
            minute_of_day >= self.start_minutes || minute_of_day < self.end_minutes
        }
    }

    fn format(&self) -> String {
        format!(
            "{:02}:{:02}-{:02}:{:02}",
            self.start_minutes / 60,
            self.start_minutes % 60,
            self.end_minutes / 60,
            self.end_minutes % 60
        )
    }
}

impl Default for BoopConfig {
    fn default() -> Self {
        Self {
            sound: true,
            title: true,
            notify: true,
            quiet: None,
            danger_patterns: DEFAULT_DANGER_PATTERNS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            notification_title: "Claude • Waiting for approval".to_string(),
            stop_title: "Claude • Done ✓".to_string(),
            notification_message: "Claude needs your attention".to_string(),
            stop_message: "Claude finished".to_string(),
        }
    }
}

impl BoopConfig {
    fn title_for(&self, event: Event) -> &str {
        match event {
            Event::Notification => &self.notification_title,
            Event::Stop => &self.stop_title,
        }
    }

    fn message_for(&self, event: Event) -> &str {
        match event {
            Event::Notification => &self.notification_message,
            Event::Stop => &self.stop_message,
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "sound": self.sound,
            "title": self.title,
            "notify": self.notify,
            "quiet": self.quiet.map(|q| q.format()).unwrap_or_default(),
            "dangerPatterns": self.danger_patterns,
            "titles": {
                "notification": self.notification_title,
                "stop": self.stop_title
            },
            "messages": {
                "notification": self.notification_message,
                "stop": self.stop_message
            }
        })
    }

    fn from_json(value: Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or("config root must be a JSON object")?;
        let defaults = Self::default();
        let quiet = match object.get("quiet") {
            Some(Value::String(raw)) if !raw.trim().is_empty() => Some(parse_quiet_window(raw)?),
            Some(Value::String(_)) | None => None,
            Some(Value::Null) => None,
            Some(_) => return Err("config.quiet must be a string like '22:00-08:00'".into()),
        };
        let danger_patterns = match object.get("dangerPatterns") {
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(|s| s.to_string())
                        .ok_or_else(|| "config.dangerPatterns must be strings".to_string())
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => defaults.danger_patterns.clone(),
            Some(_) => return Err("config.dangerPatterns must be an array of strings".into()),
        };
        Ok(Self {
            sound: bool_field(object, "sound", defaults.sound)?,
            title: bool_field(object, "title", defaults.title)?,
            notify: bool_field(object, "notify", defaults.notify)?,
            quiet,
            danger_patterns,
            notification_title: nested_string(
                object,
                "titles",
                "notification",
                defaults.notification_title,
            )?,
            stop_title: nested_string(object, "titles", "stop", defaults.stop_title)?,
            notification_message: nested_string(
                object,
                "messages",
                "notification",
                defaults.notification_message,
            )?,
            stop_message: nested_string(object, "messages", "stop", defaults.stop_message)?,
        })
    }
}

fn parse_quiet_window(raw: &str) -> Result<QuietWindow, String> {
    let (start, end) = raw
        .split_once('-')
        .ok_or_else(|| format!("quiet hours must be 'HH:MM-HH:MM', got '{raw}'"))?;
    Ok(QuietWindow {
        start_minutes: parse_hhmm(start.trim())?,
        end_minutes: parse_hhmm(end.trim())?,
    })
}

fn parse_hhmm(raw: &str) -> Result<u16, String> {
    let (h, m) = raw
        .split_once(':')
        .ok_or_else(|| format!("expected HH:MM, got '{raw}'"))?;
    let hours: u16 = h
        .parse()
        .map_err(|_| format!("invalid hours '{h}' in '{raw}'"))?;
    let minutes: u16 = m
        .parse()
        .map_err(|_| format!("invalid minutes '{m}' in '{raw}'"))?;
    if hours > 23 {
        return Err(format!("hours must be 0-23, got {hours}"));
    }
    if minutes > 59 {
        return Err(format!("minutes must be 0-59, got {minutes}"));
    }
    Ok(hours * 60 + minutes)
}

fn local_minute_of_day() -> u16 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs_since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let offset = local_utc_offset_seconds();
    let local_secs = secs_since_epoch + offset;
    let day_seconds = local_secs.rem_euclid(86_400);
    (day_seconds / 60) as u16
}

#[cfg(unix)]
fn local_utc_offset_seconds() -> i64 {
    use std::process::Command;
    Command::new("date")
        .args(["+%z"])
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8(o.stdout).ok()?;
            parse_utc_offset(s.trim())
        })
        .unwrap_or(0)
    // %z prints like -0700; if anything fails, UTC is a safe degraded mode.
}

#[cfg(windows)]
fn local_utc_offset_seconds() -> i64 {
    use std::process::Command;
    Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-Date).ToString('zzz')",
        ])
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8(o.stdout).ok()?;
            parse_iso_offset(s.trim())
        })
        .unwrap_or(0)
}

#[cfg(not(any(unix, windows)))]
fn local_utc_offset_seconds() -> i64 {
    0
}

fn parse_utc_offset(raw: &str) -> Option<i64> {
    // Accepts ±HHMM
    if raw.len() < 5 {
        return None;
    }
    let sign = match raw.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let hours: i64 = raw.get(1..3)?.parse().ok()?;
    let minutes: i64 = raw.get(3..5)?.parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

#[cfg(windows)]
fn parse_iso_offset(raw: &str) -> Option<i64> {
    // Accepts ±HH:MM
    if raw.len() < 6 {
        return None;
    }
    let sign = match raw.chars().next()? {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let hours: i64 = raw.get(1..3)?.parse().ok()?;
    let minutes: i64 = raw.get(4..6)?.parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

fn is_quiet_now(config: &BoopConfig) -> bool {
    config
        .quiet
        .map(|window| window.contains(local_minute_of_day()))
        .unwrap_or(false)
}

#[derive(Copy, Clone)]
struct Channels {
    sound: bool,
    title: bool,
    notify: bool,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Play {
            event,
            sound_only,
            no_sound,
            title,
            notify,
        } => play(event, sound_only, no_sound, title, notify),
        Cmd::Pretooluse => pretooluse(),
        Cmd::Install { visual } => install(visual),
        Cmd::Uninstall => uninstall(),
        Cmd::Config { command } => config_command(command),
        Cmd::Doctor => doctor(),
    };
    if let Err(e) = result {
        eprintln!("claude-boop: {e}");
        std::process::exit(1);
    }
}

fn play(
    event: Event,
    sound_only: bool,
    no_sound: bool,
    title_override: bool,
    notify_override: bool,
) -> Result<(), String> {
    let config = read_config()?.unwrap_or_default();
    let mut channels = Channels {
        sound: config.sound,
        title: config.title,
        notify: config.notify,
    };
    if sound_only {
        channels = Channels {
            sound: true,
            title: false,
            notify: false,
        };
    }
    if no_sound {
        channels.sound = false;
    }
    if title_override {
        channels.title = true;
    }
    if notify_override {
        channels.notify = true;
    }

    if is_quiet_now(&config) {
        channels.sound = false;
        channels.notify = false;
    }

    if channels.title {
        set_terminal_title(config.title_for(event));
    }
    if channels.notify {
        let _ = notify(event, config.message_for(event));
    }
    if channels.sound {
        let _ = play_sound(event);
    }
    Ok(())
}

fn pretooluse() -> Result<(), String> {
    use std::io::Read;
    let config = read_config()?.unwrap_or_default();
    if is_quiet_now(&config) || !config.sound {
        return Ok(());
    }

    let mut buf = String::new();
    if std::io::stdin().read_to_string(&mut buf).is_err() || buf.trim().is_empty() {
        return Ok(());
    }
    let payload: Value = match serde_json::from_str(&buf) {
        Ok(v) => v,
        Err(_) => return Ok(()),
    };
    let tool_name = payload
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !tool_name.eq_ignore_ascii_case("bash") {
        return Ok(());
    }
    let command = payload
        .pointer("/tool_input/command")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if command.is_empty() {
        return Ok(());
    }
    if !command_is_risky(command, &config.danger_patterns) {
        return Ok(());
    }
    let _ = play_danger_sound();
    Ok(())
}

fn command_is_risky(command: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .filter_map(|p| regex::RegexBuilder::new(p).case_insensitive(true).build().ok())
        .any(|re| re.is_match(command))
}

fn play_danger_sound() -> Result<(), String> {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "claude-boop-danger-{}.{}",
        std::process::id(),
        SOUND_EXT
    ));
    {
        let mut f = fs::File::create(&path).map_err(|e| format!("temp file: {e}"))?;
        f.write_all(DANGER_SOUND)
            .map_err(|e| format!("write: {e}"))?;
    }
    let status = player_command(&path);
    let _ = fs::remove_file(&path);
    status
}

fn play_sound(event: Event) -> Result<(), String> {
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

#[cfg(target_os = "windows")]
fn player_command(path: &std::path::Path) -> Result<(), String> {
    let path_str = path.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$p = New-Object Media.SoundPlayer '{}'; $p.PlaySync();",
        path_str
    );
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .map_err(|e| format!("powershell: {e}"))?;
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn player_command(_path: &std::path::Path) -> Result<(), String> {
    Err("platform not supported yet".into())
}

fn settings_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("no home dir")?;
    Ok(home.join(".claude").join("settings.json"))
}

fn config_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("no home dir")?;
    Ok(home.join(".claude").join("claude-boop.json"))
}

fn install(visual: Option<String>) -> Result<(), String> {
    let path = settings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create settings dir: {e}"))?;
    }
    ensure_config(visual)?;
    let mut root: Value = if path.exists() {
        backup_settings_once(&path)?;
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

    let exe_cmd = quote_command_part(&exe);
    let notif_cmd = format!("{exe_cmd} play --event notification");
    let stop_cmd = format!("{exe_cmd} play --event stop");
    let pretooluse_cmd = format!("{exe_cmd} pretooluse");
    add_hook(hooks, "Notification", None, &notif_cmd)?;
    add_hook(hooks, "Stop", None, &stop_cmd)?;
    add_hook(hooks, "PreToolUse", Some("Bash"), &pretooluse_cmd)?;

    let pretty = serde_json::to_string_pretty(&root).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, pretty + "\n").map_err(|e| format!("write settings: {e}"))?;
    println!("claude-boop: installed hooks in {}", path.display());
    println!("claude-boop: using binary at {exe}");
    println!("claude-boop: config at {}", config_path()?.display());
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
        for event in ["Notification", "Stop", "PreToolUse"] {
            remove_hook(hooks, event);
        }
    }
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, pretty + "\n").map_err(|e| format!("write settings: {e}"))?;
    println!("claude-boop: removed hooks from {}", path.display());
    Ok(())
}

fn add_hook(
    hooks: &mut Value,
    event: &str,
    matcher: Option<&str>,
    command: &str,
) -> Result<(), String> {
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or("settings.hooks is not an object")?;
    let matchers = hooks_obj
        .entry(event.to_string())
        .or_insert_with(|| json!([]));
    let arr = matchers
        .as_array_mut()
        .ok_or_else(|| format!("settings.hooks.{event} is not an array"))?;

    let already_present = arr.iter().any(|m| {
        m.get("hooks")
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

    let mut entry = json!({
        "hooks": [{ "type": "command", "command": command }]
    });
    if let Some(m) = matcher {
        entry
            .as_object_mut()
            .expect("entry is object")
            .insert("matcher".into(), Value::String(m.to_string()));
    }
    arr.push(entry);
    Ok(())
}

fn is_claude_boop_command(cmd: &str) -> bool {
    let trimmed = cmd.trim_start();
    let Some(first) = first_command_token(trimmed) else {
        return false;
    };
    command_basename(first)
        .map(is_claude_boop_binary)
        .unwrap_or(false)
}

fn is_claude_boop_binary(name: &str) -> bool {
    name == "claude-boop" || name.eq_ignore_ascii_case("claude-boop.exe")
}

fn command_basename(path: &str) -> Option<&str> {
    path.rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
}

fn first_command_token(command: &str) -> Option<&str> {
    let trimmed = command.trim_start();
    let mut chars = trimmed.char_indices();
    let (_, first_char) = chars.next()?;
    if first_char == '"' || first_char == '\'' {
        let start = first_char.len_utf8();
        let mut escaped = false;
        let end = trimmed[start..]
            .char_indices()
            .find_map(|(idx, ch)| {
                if escaped {
                    escaped = false;
                    return None;
                }
                if first_char == '"' && ch == '\\' {
                    escaped = true;
                    return None;
                }
                (ch == first_char).then_some(start + idx)
            })
            .unwrap_or(trimmed.len());
        return Some(&trimmed[start..end]);
    }
    let end = trimmed
        .char_indices()
        .find_map(|(idx, ch)| ch.is_whitespace().then_some(idx))
        .unwrap_or(trimmed.len());
    Some(&trimmed[..end])
}

fn quote_command_part(value: &str) -> String {
    quote_for_shell(value)
}

#[cfg(not(target_os = "windows"))]
fn quote_for_shell(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    format!("\"{escaped}\"")
}

#[cfg(target_os = "windows")]
fn quote_for_shell(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
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

fn config_command(command: ConfigCmd) -> Result<(), String> {
    match command {
        ConfigCmd::Show => {
            let config = read_config()?.unwrap_or_default();
            println!(
                "{}",
                serde_json::to_string_pretty(&config.to_json())
                    .map_err(|e| format!("serialize config: {e}"))?
            );
            Ok(())
        }
        ConfigCmd::Set { key, value } => {
            let mut config = read_config()?.unwrap_or_default();
            match key.as_str() {
                "sound" => config.sound = parse_bool(&value)?,
                "title" => config.title = parse_bool(&value)?,
                "notify" => config.notify = parse_bool(&value)?,
                "quiet" => {
                    config.quiet = if value.trim().is_empty() {
                        None
                    } else {
                        Some(parse_quiet_window(value.trim())?)
                    };
                }
                _ => {
                    return Err(
                        "config key must be sound, title, notify, or quiet".into(),
                    )
                }
            }
            write_config(&config)?;
            println!("claude-boop: set {key} to {value}");
            Ok(())
        }
        ConfigCmd::Reset => {
            write_config(&BoopConfig::default())?;
            println!("claude-boop: reset config at {}", config_path()?.display());
            Ok(())
        }
    }
}

fn doctor() -> Result<(), String> {
    let settings = settings_path()?;
    println!("settings.json: {}", exists_label(settings.exists()));
    if settings.exists() {
        match read_settings() {
            Ok(root) => {
                for event in ["Notification", "Stop", "PreToolUse"] {
                    println!(
                        "hook {event}: {}",
                        exists_label(event_hook_installed(&root, event))
                    );
                }
            }
            Err(e) => println!("hooks installed: no ({e})"),
        }
    } else {
        println!("hooks installed: no");
    }

    let config_path = config_path()?;
    let config = match read_config() {
        Ok(Some(c)) => {
            println!("config file: valid ({})", config_path.display());
            Some(c)
        }
        Ok(None) => {
            println!(
                "config file: missing, defaults will be used ({})",
                config_path.display()
            );
            None
        }
        Err(e) => {
            println!("config file: invalid ({e})");
            None
        }
    };

    let effective = config.clone().unwrap_or_default();
    match effective.quiet {
        Some(window) => println!(
            "quiet hours: {}{}",
            window.format(),
            if window.contains(local_minute_of_day()) {
                " (currently quiet)"
            } else {
                ""
            }
        ),
        None => println!("quiet hours: off"),
    }
    println!("danger patterns: {}", effective.danger_patterns.len());

    println!(
        "sound backend: {}",
        backend_label(sound_backend_available())
    );
    println!(
        "notification backend: {}",
        backend_label(notification_backend_available())
    );
    println!("terminal title: assumed available");
    Ok(())
}

fn exists_label(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}

fn backend_label(value: bool) -> &'static str {
    if value {
        "available"
    } else {
        "not found or unsupported"
    }
}

fn ensure_config(visual: Option<String>) -> Result<(), String> {
    let mut config = read_config()?.unwrap_or_default();
    if let Some(visual) = visual {
        let channels = parse_visual(&visual)?;
        config.sound = channels.sound;
        config.title = channels.title;
        config.notify = channels.notify;
    }
    write_config(&config)
}

fn read_config() -> Result<Option<BoopConfig>, String> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("read config: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(Some(BoopConfig::default()));
    }
    let value = serde_json::from_str(&raw).map_err(|e| format!("parse config: {e}"))?;
    BoopConfig::from_json(value).map(Some)
}

fn write_config(config: &BoopConfig) -> Result<(), String> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create config dir: {e}"))?;
    }
    let pretty =
        serde_json::to_string_pretty(&config.to_json()).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, pretty + "\n").map_err(|e| format!("write config: {e}"))
}

fn bool_field(
    object: &serde_json::Map<String, Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    match object.get(key) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("config.{key} must be true or false")),
        None => Ok(default),
    }
}

fn nested_string(
    object: &serde_json::Map<String, Value>,
    section: &str,
    key: &str,
    default: String,
) -> Result<String, String> {
    let Some(section_value) = object.get(section) else {
        return Ok(default);
    };
    let section_object = section_value
        .as_object()
        .ok_or_else(|| format!("config.{section} must be an object"))?;
    match section_object.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(format!("config.{section}.{key} must be a string")),
        None => Ok(default),
    }
}

fn parse_visual(raw: &str) -> Result<Channels, String> {
    let mut channels = Channels {
        sound: false,
        title: false,
        notify: false,
    };
    for part in raw
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        match part {
            "sound" => channels.sound = true,
            "title" => channels.title = true,
            "notify" => channels.notify = true,
            _ => return Err(format!("unknown visual channel '{part}'")),
        }
    }
    Ok(channels)
}

fn parse_bool(raw: &str) -> Result<bool, String> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("config value must be true or false".into()),
    }
}

fn backup_settings_once(path: &std::path::Path) -> Result<(), String> {
    let backup = path.with_extension("json.claude-boop.bak");
    if !backup.exists() {
        fs::copy(path, &backup).map_err(|e| format!("backup settings: {e}"))?;
    }
    Ok(())
}

fn read_settings() -> Result<Value, String> {
    let path = settings_path()?;
    let raw = fs::read_to_string(&path).map_err(|e| format!("read settings: {e}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("parse settings.json: {e}"))
}

fn event_hook_installed(root: &Value, event: &str) -> bool {
    let Some(hooks) = root.get("hooks").and_then(|h| h.as_object()) else {
        return false;
    };
    hooks
        .get(event)
        .and_then(|v| v.as_array())
        .map(|matchers| {
            matchers.iter().any(|matcher| {
                matcher
                    .get("hooks")
                    .and_then(|h| h.as_array())
                    .map(|inner| {
                        inner.iter().any(|h| {
                            h.get("command")
                                .and_then(|c| c.as_str())
                                .map(is_claude_boop_command)
                                .unwrap_or(false)
                        })
                    })
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
fn hooks_installed(root: &Value) -> bool {
    ["Notification", "Stop", "PreToolUse"]
        .iter()
        .all(|event| event_hook_installed(root, event))
}

fn set_terminal_title(title: &str) {
    eprint!("\x1b]0;{title}\x07");
}

fn notify(event: Event, message: &str) -> Result<bool, String> {
    native_notify(event, message)
}

#[cfg(target_os = "macos")]
fn native_notify(_event: Event, message: &str) -> Result<bool, String> {
    if !command_available("osascript") {
        return Ok(false);
    }
    let script = format!(
        "display notification {} with title \"Claude\"",
        apple_script_string(message)
    );
    let status = Command::new("osascript")
        .args(["-e", &script])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("osascript: {e}"))?;
    Ok(status.success())
}

#[cfg(target_os = "linux")]
fn native_notify(_event: Event, message: &str) -> Result<bool, String> {
    if !command_available("notify-send") {
        return Ok(false);
    }
    let status = Command::new("notify-send")
        .args(["Claude", message])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("notify-send: {e}"))?;
    Ok(status.success())
}

#[cfg(target_os = "windows")]
fn native_notify(_event: Event, _message: &str) -> Result<bool, String> {
    Ok(false)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn native_notify(_event: Event, _message: &str) -> Result<bool, String> {
    Ok(false)
}

#[cfg(target_os = "macos")]
fn apple_script_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(target_os = "macos")]
fn sound_backend_available() -> bool {
    command_available("afplay")
}

#[cfg(target_os = "linux")]
fn sound_backend_available() -> bool {
    ["paplay", "aplay", "ffplay"]
        .iter()
        .any(|cmd| command_available(cmd))
}

#[cfg(target_os = "windows")]
fn sound_backend_available() -> bool {
    command_available("powershell") || command_available("powershell.exe")
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn sound_backend_available() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn notification_backend_available() -> bool {
    command_available("osascript")
}

#[cfg(target_os = "linux")]
fn notification_backend_available() -> bool {
    command_available("notify-send")
}

#[cfg(target_os = "windows")]
fn notification_backend_available() -> bool {
    false
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn notification_backend_available() -> bool {
    false
}

fn command_available(cmd: &str) -> bool {
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };

    #[cfg(target_os = "windows")]
    let candidates: Vec<String> = {
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let mut names = vec![cmd.to_string()];
        if std::path::Path::new(cmd).extension().is_none() {
            names.extend(
                pathext
                    .split(';')
                    .filter(|ext| !ext.is_empty())
                    .map(|ext| format!("{cmd}{ext}")),
            );
        }
        names
    };

    #[cfg(not(target_os = "windows"))]
    let candidates = vec![cmd.to_string()];

    std::env::split_paths(&paths).any(|dir| {
        candidates
            .iter()
            .any(|name| command_candidate_available(&dir.join(name)))
    })
}

#[cfg(unix)]
fn command_candidate_available(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.is_file()
        && path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
fn command_candidate_available(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_with_defaults() {
        let config = BoopConfig::default();
        assert_eq!(BoopConfig::from_json(config.to_json()).unwrap(), config);
    }

    #[test]
    fn config_accepts_missing_optional_fields() {
        let config = BoopConfig::from_json(json!({ "sound": false })).unwrap();
        assert!(!config.sound);
        assert!(config.title);
        assert!(config.notify);
        assert_eq!(
            config.title_for(Event::Notification),
            "Claude • Waiting for approval"
        );
    }

    #[test]
    fn visual_parser_selects_only_requested_channels() {
        let channels = parse_visual("sound,title").unwrap();
        assert!(channels.sound);
        assert!(channels.title);
        assert!(!channels.notify);
    }

    #[test]
    fn hook_detection_matches_absolute_binary_paths() {
        assert!(is_claude_boop_command(
            "/usr/local/bin/claude-boop play --event notification"
        ));
        assert!(is_claude_boop_command(
            "\"/Applications/My Tools/claude-boop\" play --event notification"
        ));
        assert!(is_claude_boop_command(
            r#""C:\Program Files\claude-boop\claude-boop.exe" play --event stop"#
        ));
        assert!(is_claude_boop_command("claude-boop play --event stop"));
        assert!(is_claude_boop_command("claude-boop.exe play --event stop"));
        assert!(!is_claude_boop_command("echo claude-boop"));
    }

    #[test]
    fn quote_command_part_handles_spaces() {
        assert_eq!(
            quote_command_part("/usr/local/bin/claude-boop"),
            "\"/usr/local/bin/claude-boop\""
        );

        let quoted = quote_command_part("/Users/example/My Tools/claude-boop");
        assert!(quoted.starts_with('"') || quoted.starts_with('\''));
        assert!(quoted.ends_with('"') || quoted.ends_with('\''));
    }

    #[test]
    fn hook_installed_requires_all_three_events() {
        let root = json!({
            "hooks": {
                "Notification": [{ "hooks": [{ "type": "command", "command": "claude-boop play --event notification" }] }],
                "Stop": [{ "hooks": [{ "type": "command", "command": "claude-boop play --event stop" }] }],
                "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "claude-boop pretooluse" }] }]
            }
        });
        assert!(hooks_installed(&root));
    }

    #[test]
    fn hook_installed_false_when_pretooluse_missing() {
        let root = json!({
            "hooks": {
                "Notification": [{ "hooks": [{ "type": "command", "command": "claude-boop play --event notification" }] }],
                "Stop": [{ "hooks": [{ "type": "command", "command": "claude-boop play --event stop" }] }]
            }
        });
        assert!(!hooks_installed(&root));
        assert!(event_hook_installed(&root, "Notification"));
        assert!(!event_hook_installed(&root, "PreToolUse"));
    }

    #[test]
    fn add_hook_is_idempotent() {
        let mut hooks = json!({});
        add_hook(
            &mut hooks,
            "Notification",
            None,
            "claude-boop play --event notification",
        )
        .unwrap();
        add_hook(
            &mut hooks,
            "Notification",
            None,
            "claude-boop play --event notification",
        )
        .unwrap();

        let count = hooks["Notification"].as_array().unwrap().len();
        assert_eq!(count, 1);
    }

    #[test]
    fn add_hook_includes_matcher_when_given() {
        let mut hooks = json!({});
        add_hook(&mut hooks, "PreToolUse", Some("Bash"), "claude-boop pretooluse").unwrap();
        let entry = &hooks["PreToolUse"][0];
        assert_eq!(entry["matcher"].as_str(), Some("Bash"));
        assert_eq!(
            entry["hooks"][0]["command"].as_str(),
            Some("claude-boop pretooluse")
        );
    }

    #[test]
    fn remove_hook_preserves_non_boop_hooks() {
        let mut hooks = json!({
            "Notification": [
                { "hooks": [{ "type": "command", "command": "claude-boop play --event notification" }] },
                { "hooks": [{ "type": "command", "command": "echo keep-me" }] }
            ]
        });
        let object = hooks.as_object_mut().unwrap();

        remove_hook(object, "Notification");

        let remaining = object["Notification"].as_array().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0]["hooks"][0]["command"].as_str().unwrap(),
            "echo keep-me"
        );
    }

    #[test]
    fn quiet_window_handles_overnight_range() {
        let window = parse_quiet_window("22:00-08:00").unwrap();
        assert!(window.contains(23 * 60));
        assert!(window.contains(2 * 60));
        assert!(window.contains(7 * 60 + 59));
        assert!(!window.contains(8 * 60));
        assert!(!window.contains(15 * 60));
    }

    #[test]
    fn quiet_window_handles_same_day_range() {
        let window = parse_quiet_window("13:00-17:30").unwrap();
        assert!(window.contains(13 * 60));
        assert!(window.contains(17 * 60));
        assert!(!window.contains(17 * 60 + 30));
        assert!(!window.contains(12 * 60 + 59));
    }

    #[test]
    fn quiet_window_rejects_invalid_input() {
        assert!(parse_quiet_window("25:00-08:00").is_err());
        assert!(parse_quiet_window("22:00").is_err());
        assert!(parse_quiet_window("22:60-08:00").is_err());
    }

    #[test]
    fn config_round_trips_quiet_hours() {
        let mut config = BoopConfig::default();
        config.quiet = Some(parse_quiet_window("22:00-08:00").unwrap());
        let parsed = BoopConfig::from_json(config.to_json()).unwrap();
        assert_eq!(parsed.quiet, config.quiet);
    }

    #[test]
    fn config_treats_empty_quiet_as_none() {
        let config = BoopConfig::from_json(json!({ "quiet": "" })).unwrap();
        assert!(config.quiet.is_none());
    }

    #[test]
    fn default_danger_patterns_all_compile() {
        for pattern in DEFAULT_DANGER_PATTERNS {
            regex::RegexBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .unwrap_or_else(|e| panic!("default pattern {pattern:?} did not compile: {e}"));
        }
    }

    #[test]
    fn danger_patterns_match_dangerous_commands() {
        let defaults: Vec<String> = DEFAULT_DANGER_PATTERNS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(command_is_risky("rm -rf /tmp/foo", &defaults));
        assert!(command_is_risky("RM -RF /tmp/foo", &defaults));
        assert!(command_is_risky("git push --force origin main", &defaults));
        assert!(command_is_risky("git push -f origin main", &defaults));
        assert!(command_is_risky("git reset --hard HEAD~3", &defaults));
        assert!(command_is_risky("sudo apt update", &defaults));
        assert!(command_is_risky("curl https://x.sh | sh", &defaults));
        assert!(command_is_risky("dd if=/dev/zero of=/dev/sda", &defaults));
    }

    #[test]
    fn danger_patterns_skip_safe_commands() {
        let defaults: Vec<String> = DEFAULT_DANGER_PATTERNS
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!command_is_risky("ls -la", &defaults));
        assert!(!command_is_risky("npm install", &defaults));
        assert!(!command_is_risky("git push origin main", &defaults));
        assert!(!command_is_risky("echo 'rm -rf is dangerous'", &defaults));
        assert!(!command_is_risky("git log --all", &defaults));
        // "rm" alone without -r/-f shouldn't fire
        assert!(!command_is_risky("rm single-file.txt", &defaults));
    }
}
