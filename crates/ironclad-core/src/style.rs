use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

impl ColorMode {
    pub fn from_flag(s: &str) -> Self {
        match s {
            "always" => Self::Always,
            "never" => Self::Never,
            _ => Self::Auto,
        }
    }
}

/// CRT-style CLI theme with phosphor green palette and WarGames typewriter effects.
///
/// Precedence: `--color` flag > `NO_COLOR` env var > TTY auto-detection.
/// Draw (typewriter) is enabled by default on interactive TTY, disabled with `--no-draw`.
#[derive(Debug, Clone)]
pub struct Theme {
    enabled: bool,
    draw: bool,
}

impl Theme {
    pub fn detect() -> Self {
        Self::resolve(ColorMode::Auto)
    }

    pub fn from_flag(flag: &str) -> Self {
        Self::resolve(ColorMode::from_flag(flag))
    }

    pub fn resolve(mode: ColorMode) -> Self {
        let enabled = match mode {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => {
                let no_color = std::env::var("NO_COLOR")
                    .map(|v| !v.is_empty())
                    .unwrap_or(false);
                if no_color {
                    false
                } else {
                    std::io::stderr().is_terminal()
                }
            }
        };
        Self { enabled, draw: enabled }
    }

    pub fn plain() -> Self {
        Self { enabled: false, draw: false }
    }

    pub fn with_draw(mut self, draw: bool) -> Self {
        self.draw = draw;
        self
    }

    pub fn colors_enabled(&self) -> bool {
        self.enabled
    }

    pub fn draw_enabled(&self) -> bool {
        self.draw
    }

    // ── CRT Phosphor Green Palette ───────────────────────────────

    /// Bright phosphor green (256-color 46). Banner, headings, emphasis.
    pub fn accent(&self) -> &'static str {
        if self.enabled { "\x1b[38;5;46m" } else { "" }
    }

    /// Body-text phosphor green (256-color 40). Metadata, step counters, secondary text.
    pub fn dim(&self) -> &'static str {
        if self.enabled { "\x1b[38;5;40m" } else { "" }
    }

    /// Bright phosphor green (256-color 46). IDs, paths, code values.
    pub fn mono(&self) -> &'static str {
        if self.enabled { "\x1b[38;5;46m" } else { "" }
    }

    /// Bright green. Explicit "all passed" summaries, enabled/active states.
    pub fn success(&self) -> &'static str {
        if self.enabled { "\x1b[92m" } else { "" }
    }

    /// Bright yellow. Warnings, fallback states, skipped items.
    pub fn warn(&self) -> &'static str {
        if self.enabled { "\x1b[93m" } else { "" }
    }

    /// Bright red. Errors, failures, disabled states.
    pub fn error(&self) -> &'static str {
        if self.enabled { "\x1b[91m" } else { "" }
    }

    /// Bright cyan. Auto-fix actions, debug info, discovery output.
    pub fn info(&self) -> &'static str {
        if self.enabled { "\x1b[96m" } else { "" }
    }


    // ── Typography modifiers ─────────────────────────────────────

    pub fn bold(&self) -> &'static str {
        if self.enabled { "\x1b[1m" } else { "" }
    }

    /// Soft reset: clears styles, sets CRT green body text color.
    pub fn reset(&self) -> &'static str {
        if self.enabled { "\x1b[0m\x1b[38;5;40m" } else { "" }
    }

    /// Hard reset: returns terminal to default colors. Use at program exit.
    pub fn hard_reset(&self) -> &'static str {
        if self.enabled { "\x1b[0m" } else { "" }
    }

    // ── CRT Typewriter Effects ───────────────────────────────────

    /// Typewrite to stderr, character-by-character. ANSI sequences emitted instantly.
    /// Skips delay when draw is disabled (instant print).
    pub fn typewrite(&self, text: &str, delay_ms: u64) {
        use std::io::Write;
        if !self.draw {
            eprint!("{text}");
            return;
        }
        let delay = std::time::Duration::from_millis(delay_ms);
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                let mut seq = String::from(ch);
                for c in chars.by_ref() {
                    seq.push(c);
                    if c == 'm' { break; }
                }
                eprint!("{seq}");
            } else if ch == '\n' {
                eprintln!();
            } else {
                eprint!("{ch}");
                std::io::stderr().flush().ok();
                std::thread::sleep(delay);
            }
        }
    }

    /// Typewrite to stderr + newline.
    pub fn typewrite_line(&self, text: &str, delay_ms: u64) {
        self.typewrite(text, delay_ms);
        eprintln!();
    }

    /// Typewrite to stdout, character-by-character.
    pub fn typewrite_stdout(&self, text: &str, delay_ms: u64) {
        use std::io::Write;
        if !self.draw {
            print!("{text}");
            return;
        }
        let delay = std::time::Duration::from_millis(delay_ms);
        let mut chars = text.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                let mut seq = String::from(ch);
                for c in chars.by_ref() {
                    seq.push(c);
                    if c == 'm' { break; }
                }
                print!("{seq}");
            } else if ch == '\n' {
                println!();
            } else {
                print!("{ch}");
                std::io::stdout().flush().ok();
                std::thread::sleep(delay);
            }
        }
    }

    /// Typewrite to stdout + newline.
    pub fn typewrite_line_stdout(&self, text: &str, delay_ms: u64) {
        use std::io::Write;
        self.typewrite_stdout(text, delay_ms);
        println!();
        std::io::stdout().flush().ok();
    }

    /// Start a looping keystroke sound in the background (macOS only).
    /// Returns a handle that stops the sound when dropped.
    pub fn start_typing_sound(&self) -> Option<SoundHandle> {
        if !self.draw {
            return None;
        }
        #[cfg(target_os = "macos")]
        {
            let sound = "/System/Library/Sounds/Tink.aiff";
            if std::path::Path::new(sound).exists() {
                let child = std::process::Command::new("bash")
                    .args(["-c", &format!(
                        "while true; do afplay -t 0.04 {sound} 2>/dev/null; sleep 0.04; done"
                    )])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .ok()?;
                return Some(SoundHandle(child));
            }
        }
        None
    }
}

/// RAII guard that stops a background sound process when dropped.
pub struct SoundHandle(std::process::Child);

impl Drop for SoundHandle {
    fn drop(&mut self) {
        self.0.kill().ok();
        self.0.wait().ok();
    }
}

/// Convenience sleep in milliseconds.
pub fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_theme_returns_empty_strings() {
        let t = Theme::plain();
        assert!(!t.colors_enabled());
        assert!(!t.draw_enabled());
        assert_eq!(t.accent(), "");
        assert_eq!(t.dim(), "");
        assert_eq!(t.mono(), "");
        assert_eq!(t.success(), "");
        assert_eq!(t.warn(), "");
        assert_eq!(t.error(), "");
        assert_eq!(t.info(), "");
        assert_eq!(t.bold(), "");
        assert_eq!(t.reset(), "");
        assert_eq!(t.hard_reset(), "");
    }

    #[test]
    fn always_mode_forces_color_and_draw() {
        let t = Theme::resolve(ColorMode::Always);
        assert!(t.colors_enabled());
        assert!(t.draw_enabled());
        assert!(!t.accent().is_empty());
        assert!(!t.reset().is_empty());
    }

    #[test]
    fn with_draw_false_disables_draw() {
        let t = Theme::resolve(ColorMode::Always).with_draw(false);
        assert!(t.colors_enabled());
        assert!(!t.draw_enabled());
    }

    #[test]
    fn reset_includes_crt_green() {
        let t = Theme::resolve(ColorMode::Always);
        let r = t.reset();
        assert!(r.contains("\x1b[0m"), "reset should clear styles");
        assert!(r.contains("\x1b[38;5;40m"), "reset should set CRT green");
    }

    #[test]
    fn hard_reset_is_plain() {
        let t = Theme::resolve(ColorMode::Always);
        assert_eq!(t.hard_reset(), "\x1b[0m");
    }

    #[test]
    fn never_mode_disables_everything() {
        let t = Theme::resolve(ColorMode::Never);
        assert!(!t.colors_enabled());
        assert!(!t.draw_enabled());
        assert_eq!(t.accent(), "");
    }

    #[test]
    fn from_flag_parses_correctly() {
        assert_eq!(ColorMode::from_flag("always"), ColorMode::Always);
        assert_eq!(ColorMode::from_flag("never"), ColorMode::Never);
        assert_eq!(ColorMode::from_flag("auto"), ColorMode::Auto);
        assert_eq!(ColorMode::from_flag("garbage"), ColorMode::Auto);
    }
}
