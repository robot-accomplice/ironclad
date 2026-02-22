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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeVariant {
    CrtGreen,
    CrtOrange,
    Terminal,
}

impl ThemeVariant {
    pub fn from_flag(s: &str) -> Self {
        match s {
            "crt-orange" => Self::CrtOrange,
            "terminal" => Self::Terminal,
            _ => Self::CrtGreen,
        }
    }
}

/// CLI theme with selectable color palettes and optional typewriter effects.
///
/// Precedence: `--color` flag > `NO_COLOR` env var > TTY auto-detection.
/// Draw (typewriter) is enabled by default on interactive TTY, disabled with `--no-draw`.
#[derive(Debug, Clone)]
pub struct Theme {
    enabled: bool,
    draw: bool,
    variant: ThemeVariant,
    nerdmode: bool,
}

impl Theme {
    pub fn detect() -> Self {
        Self::resolve(ColorMode::Auto, ThemeVariant::CrtGreen)
    }

    pub fn from_flags(color_flag: &str, theme_flag: &str) -> Self {
        Self::resolve(ColorMode::from_flag(color_flag), ThemeVariant::from_flag(theme_flag))
    }

    pub fn resolve(mode: ColorMode, variant: ThemeVariant) -> Self {
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
        Self { enabled, draw: enabled, variant, nerdmode: false }
    }

    pub fn plain() -> Self {
        Self { enabled: false, draw: false, variant: ThemeVariant::CrtGreen, nerdmode: false }
    }

    pub fn with_draw(mut self, draw: bool) -> Self {
        self.draw = draw;
        self
    }

    pub fn with_nerdmode(mut self, nerd: bool) -> Self {
        if nerd {
            self.nerdmode = true;
            self.draw = true;
            if self.variant == ThemeVariant::Terminal {
                self.variant = ThemeVariant::CrtGreen;
            }
        }
        self
    }

    pub fn colors_enabled(&self) -> bool {
        self.enabled
    }

    pub fn draw_enabled(&self) -> bool {
        self.draw
    }

    pub fn variant(&self) -> ThemeVariant {
        self.variant
    }

    pub fn nerdmode(&self) -> bool {
        self.nerdmode
    }

    // ── Icon Accessors ───────────────────────────────────────────
    // Return emoji by default; ASCII when nerdmode is active.

    pub fn icon_ok(&self) -> &'static str {
        if self.nerdmode { "[OK]" } else { "\u{2705}" }
    }

    pub fn icon_action(&self) -> &'static str {
        if self.nerdmode { "[>>]" } else { "\u{26a1}" }
    }

    pub fn icon_warn(&self) -> &'static str {
        if self.nerdmode { "[!!]" } else { "\u{26a0}\u{fe0f}" }
    }

    pub fn icon_detail(&self) -> &'static str {
        if self.nerdmode { ">" } else { "\u{25b8}" }
    }

    pub fn icon_error(&self) -> &'static str {
        if self.nerdmode { "[XX]" } else { "\u{26d3}" }
    }

    // ── Color Palette ────────────────────────────────────────────

    /// Emphasis/highlight color. Bright green, bright orange, or bold depending on variant.
    pub fn accent(&self) -> &'static str {
        if !self.enabled { return ""; }
        match self.variant {
            ThemeVariant::CrtGreen  => "\x1b[38;5;46m",
            ThemeVariant::CrtOrange => "\x1b[38;5;208m",
            ThemeVariant::Terminal  => "\x1b[1m",
        }
    }

    /// Body-text color. Matches the variant's base tone; empty for Terminal.
    pub fn dim(&self) -> &'static str {
        if !self.enabled { return ""; }
        match self.variant {
            ThemeVariant::CrtGreen  => "\x1b[38;5;40m",
            ThemeVariant::CrtOrange => "\x1b[38;5;172m",
            ThemeVariant::Terminal  => "",
        }
    }

    /// Monospace-value color. Same as accent for CRT variants; bold for Terminal.
    pub fn mono(&self) -> &'static str {
        if !self.enabled { return ""; }
        match self.variant {
            ThemeVariant::CrtGreen  => "\x1b[38;5;46m",
            ThemeVariant::CrtOrange => "\x1b[38;5;208m",
            ThemeVariant::Terminal  => "\x1b[1m",
        }
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

    /// Soft reset: clears styles and re-tints to the variant's body color.
    /// For Terminal variant, this is a plain reset (no tint).
    pub fn reset(&self) -> &'static str {
        if !self.enabled { return ""; }
        match self.variant {
            ThemeVariant::CrtGreen  => "\x1b[0m\x1b[38;5;40m",
            ThemeVariant::CrtOrange => "\x1b[0m\x1b[38;5;172m",
            ThemeVariant::Terminal  => "\x1b[0m",
        }
    }

    /// Hard reset: returns terminal to default colors. Use at program exit.
    pub fn hard_reset(&self) -> &'static str {
        if self.enabled { "\x1b[0m" } else { "" }
    }

    // ── Typewriter Effects ───────────────────────────────────────

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
        let t = Theme::resolve(ColorMode::Always, ThemeVariant::CrtGreen);
        assert!(t.colors_enabled());
        assert!(t.draw_enabled());
        assert!(!t.accent().is_empty());
        assert!(!t.reset().is_empty());
    }

    #[test]
    fn with_draw_false_disables_draw() {
        let t = Theme::resolve(ColorMode::Always, ThemeVariant::CrtGreen).with_draw(false);
        assert!(t.colors_enabled());
        assert!(!t.draw_enabled());
    }

    #[test]
    fn crt_green_reset_includes_green_tint() {
        let t = Theme::resolve(ColorMode::Always, ThemeVariant::CrtGreen);
        let r = t.reset();
        assert!(r.contains("\x1b[0m"), "reset should clear styles");
        assert!(r.contains("\x1b[38;5;40m"), "CrtGreen reset should tint green");
    }

    #[test]
    fn crt_orange_palette() {
        let t = Theme::resolve(ColorMode::Always, ThemeVariant::CrtOrange);
        assert!(t.accent().contains("208"), "CrtOrange accent should be 208");
        assert!(t.dim().contains("172"), "CrtOrange dim should be 172");
        assert!(t.reset().contains("172"), "CrtOrange reset should tint orange");
    }

    #[test]
    fn terminal_variant_no_tint() {
        let t = Theme::resolve(ColorMode::Always, ThemeVariant::Terminal);
        assert_eq!(t.reset(), "\x1b[0m", "Terminal reset should be plain");
        assert_eq!(t.dim(), "", "Terminal dim should be empty");
        assert_eq!(t.accent(), "\x1b[1m", "Terminal accent should be bold");
    }

    #[test]
    fn hard_reset_is_plain() {
        let t = Theme::resolve(ColorMode::Always, ThemeVariant::CrtGreen);
        assert_eq!(t.hard_reset(), "\x1b[0m");
    }

    #[test]
    fn never_mode_disables_everything() {
        let t = Theme::resolve(ColorMode::Never, ThemeVariant::CrtGreen);
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

    #[test]
    fn theme_variant_from_flag() {
        assert_eq!(ThemeVariant::from_flag("crt-green"), ThemeVariant::CrtGreen);
        assert_eq!(ThemeVariant::from_flag("crt-orange"), ThemeVariant::CrtOrange);
        assert_eq!(ThemeVariant::from_flag("terminal"), ThemeVariant::Terminal);
        assert_eq!(ThemeVariant::from_flag("garbage"), ThemeVariant::CrtGreen);
    }

    #[test]
    fn semantic_colors_same_across_variants() {
        let green = Theme::resolve(ColorMode::Always, ThemeVariant::CrtGreen);
        let orange = Theme::resolve(ColorMode::Always, ThemeVariant::CrtOrange);
        let term = Theme::resolve(ColorMode::Always, ThemeVariant::Terminal);

        assert_eq!(green.success(), orange.success());
        assert_eq!(green.success(), term.success());
        assert_eq!(green.warn(), orange.warn());
        assert_eq!(green.error(), orange.error());
        assert_eq!(green.info(), orange.info());
    }

    #[test]
    fn nerdmode_forces_ascii_icons() {
        let t = Theme::resolve(ColorMode::Always, ThemeVariant::CrtGreen).with_nerdmode(true);
        assert_eq!(t.icon_ok(), "[OK]");
        assert_eq!(t.icon_action(), "[>>]");
        assert_eq!(t.icon_warn(), "[!!]");
        assert_eq!(t.icon_detail(), ">");
        assert_eq!(t.icon_error(), "[XX]");
    }

    #[test]
    fn nerdmode_overrides_terminal_to_green() {
        let t = Theme::resolve(ColorMode::Always, ThemeVariant::Terminal).with_nerdmode(true);
        assert_eq!(t.variant(), ThemeVariant::CrtGreen);
        assert!(t.reset().contains("\x1b[38;5;40m"));
    }

    #[test]
    fn nerdmode_respects_orange() {
        let t = Theme::resolve(ColorMode::Always, ThemeVariant::CrtOrange).with_nerdmode(true);
        assert_eq!(t.variant(), ThemeVariant::CrtOrange);
        assert!(t.accent().contains("208"));
    }

    #[test]
    fn nerdmode_forces_draw() {
        let t = Theme::resolve(ColorMode::Always, ThemeVariant::CrtGreen)
            .with_draw(false)
            .with_nerdmode(true);
        assert!(t.draw_enabled());
    }

    #[test]
    fn default_icons_are_emoji() {
        let t = Theme::resolve(ColorMode::Always, ThemeVariant::CrtGreen);
        assert!(!t.nerdmode());
        assert_eq!(t.icon_ok(), "\u{2705}");
        assert_eq!(t.icon_action(), "\u{26a1}");
        assert_eq!(t.icon_warn(), "\u{26a0}\u{fe0f}");
        assert_eq!(t.icon_detail(), "\u{25b8}");
        assert_eq!(t.icon_error(), "\u{26d3}");
    }
}
