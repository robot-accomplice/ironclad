//! Channel-specific output formatting for human-readable delivery.
//!
//! Each channel has different formatting capabilities. The LLM produces standard
//! Markdown, and each [`ChannelFormatter`] implementation converts that into
//! the channel's native formatting language for maximum readability.
//!
//! ## Channel Capabilities
//!
//! | Channel   | Bold | Italic | Code | Headers | Links |
//! |-----------|------|--------|------|---------|-------|
//! | Telegram  | ✓    | ✓      | ✓    | mapped  | ✓     |
//! | Discord   | ✓    | ✓      | ✓    | ✓       | ✓     |
//! | WhatsApp  | ✓    | ✓      | ✓    | mapped  | bare  |
//! | Signal    | —    | —      | —    | —       | bare  |
//! | Email     | ✓    | ✓      | ✓    | ✓       | ✓     |
//! | Web       | ✓    | ✓      | ✓    | ✓       | ✓     |

use std::borrow::Cow;

/// Trait for converting LLM Markdown output into channel-native formatting.
///
/// Each channel implementation transforms standard Markdown into whatever
/// formatting the delivery platform supports, preserving as much structure
/// and readability as the channel allows.
pub trait ChannelFormatter: Send + Sync {
    /// The platform identifier this formatter handles (e.g., "telegram").
    fn platform(&self) -> &str;

    /// Format LLM Markdown output for delivery on this channel.
    ///
    /// The input `content` is raw LLM output (standard Markdown with possible
    /// internal metadata lines). The output is channel-native formatted text
    /// ready for the channel adapter's `send()` method.
    fn format(&self, content: &str) -> String;
}

// ── Shared utilities ────────────────────────────────────────────────────

/// Strip internal delegation/orchestration metadata lines that should never
/// reach end users. This runs before any channel-specific formatting.
fn strip_internal_metadata(content: &str) -> String {
    content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            // Filter lines that leak internal orchestration
            !trimmed.starts_with("[Delegated to ")
                && !trimmed.starts_with("[Delegation from ")
                && !trimmed.starts_with("[Tool call: ")
                && !trimmed.starts_with("[Tool result: ")
                && !trimmed.starts_with("---orchestration")
                && !trimmed.starts_with("[Internal:")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Strip numeric bracket citations like `[1]`, `[23]` that appear in LLM
/// output as source references — most channels can't render them usefully.
fn strip_bracket_citations(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '[' {
            let mut j = i + 1;
            let mut has_digit = false;
            while j < chars.len() && chars[j].is_ascii_digit() {
                has_digit = true;
                j += 1;
            }
            if has_digit && j < chars.len() && chars[j] == ']' {
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Collapse repeated blank lines to at most one.
fn collapse_blank_lines(text: &str) -> String {
    let mut out = Vec::new();
    let mut prev_blank = false;
    for line in text.lines() {
        let is_blank = line.trim().is_empty();
        if is_blank && prev_blank {
            continue;
        }
        out.push(line);
        prev_blank = is_blank;
    }
    out.join("\n")
}

// ── Telegram ────────────────────────────────────────────────────────────

/// Telegram MarkdownV2 formatter.
///
/// Telegram supports MarkdownV2 with its own escaping rules. This formatter
/// converts standard Markdown into Telegram MarkdownV2, preserving bold,
/// italic, code, and code blocks while properly escaping special characters.
///
/// Telegram MarkdownV2 special chars that must be escaped outside of code:
/// `_`, `*`, `[`, `]`, `(`, `)`, `~`, `` ` ``, `>`, `#`, `+`, `-`, `=`, `|`, `{`, `}`, `.`, `!`
pub struct TelegramFormatter;

impl TelegramFormatter {
    /// Characters that must be escaped in Telegram MarkdownV2 text segments.
    const ESCAPE_CHARS: &[char] = &[
        '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ];

    /// Escape a plain-text segment for Telegram MarkdownV2.
    fn escape_text(text: &str) -> String {
        let mut out = String::with_capacity(text.len() * 2);
        for ch in text.chars() {
            if Self::ESCAPE_CHARS.contains(&ch) {
                out.push('\\');
            }
            out.push(ch);
        }
        out
    }

    /// Convert a single line of Markdown to Telegram MarkdownV2.
    ///
    /// Handles: **bold** → *bold*, *italic*/_italic_/__italic__ → _italic_,
    /// ~~strikethrough~~ → ~strikethrough~, `inline code` → `inline code`,
    /// [text](url) → [text](url), > blockquotes.
    /// Markdown headers (# / ## / ###) are converted to bold lines.
    fn convert_line(line: &str) -> String {
        let trimmed = line.trim();

        // Convert Markdown headers to bold text
        if let Some(rest) = trimmed.strip_prefix("### ") {
            return format!("*{}*", Self::escape_text(rest.trim()));
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            return format!("*{}*", Self::escape_text(rest.trim()));
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return format!("*{}*", Self::escape_text(rest.trim()));
        }

        // Blockquote: > text → >text (Telegram MarkdownV2 blockquote)
        if let Some(rest) = trimmed.strip_prefix("> ") {
            return format!(">{}", Self::convert_inline(rest));
        }
        if trimmed == ">" {
            return ">".to_string();
        }

        // Process inline formatting
        Self::convert_inline(trimmed)
    }

    /// Convert inline Markdown formatting to Telegram MarkdownV2.
    fn convert_inline(text: &str) -> String {
        let mut result = String::with_capacity(text.len() * 2);
        let chars: Vec<char> = text.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            // Inline code: `code`
            if chars[i] == '`'
                && i + 1 < len
                && let Some(end) = find_closing(&chars, i + 1, '`')
            {
                let code_text: String = chars[i + 1..end].iter().collect();
                result.push('`');
                result.push_str(&code_text); // no escaping inside code
                result.push('`');
                i = end + 1;
                continue;
            }

            // Bold: **text** → *text*
            if i + 1 < len
                && chars[i] == '*'
                && chars[i + 1] == '*'
                && let Some(end) = find_double_closing(&chars, i + 2, '*')
            {
                let inner: String = chars[i + 2..end].iter().collect();
                result.push('*');
                result.push_str(&Self::escape_text(&inner));
                result.push('*');
                i = end + 2;
                continue;
            }

            // Strikethrough: ~~text~~ → ~text~
            if i + 1 < len
                && chars[i] == '~'
                && chars[i + 1] == '~'
                && let Some(end) = find_double_closing(&chars, i + 2, '~')
            {
                let inner: String = chars[i + 2..end].iter().collect();
                result.push('~');
                result.push_str(&Self::escape_text(&inner));
                result.push('~');
                i = end + 2;
                continue;
            }

            // Single-tilde strikethrough: ~text~ → ~text~ (already Telegram-native;
            // LLMs sometimes output this instead of ~~text~~)
            if chars[i] == '~'
                && (i == 0 || chars[i - 1] != '~')
                && i + 1 < len
                && chars[i + 1] != '~'
                && let Some(end) = find_closing_not_doubled(&chars, i + 1, '~')
            {
                let inner: String = chars[i + 1..end].iter().collect();
                result.push('~');
                result.push_str(&Self::escape_text(&inner));
                result.push('~');
                i = end + 1;
                continue;
            }

            // Italic: *text* (single) or _text_
            if chars[i] == '*'
                && (i == 0 || chars[i - 1] != '*')
                && i + 1 < len
                && chars[i + 1] != '*'
                && let Some(end) = find_closing_not_doubled(&chars, i + 1, '*')
            {
                let inner: String = chars[i + 1..end].iter().collect();
                result.push('_');
                result.push_str(&Self::escape_text(&inner));
                result.push('_');
                i = end + 1;
                continue;
            }

            // Italic: __text__ → _text_
            if i + 1 < len
                && chars[i] == '_'
                && chars[i + 1] == '_'
                && let Some(end) = find_double_closing(&chars, i + 2, '_')
            {
                let inner: String = chars[i + 2..end].iter().collect();
                result.push_str("__");
                result.push_str(&Self::escape_text(&inner));
                result.push_str("__");
                i = end + 2;
                continue;
            }

            // Italic: _text_ (single underscores) → _text_
            if chars[i] == '_'
                && (i == 0 || chars[i - 1] != '_')
                && i + 1 < len
                && chars[i + 1] != '_'
                && let Some(end) = find_closing_not_doubled(&chars, i + 1, '_')
            {
                let inner: String = chars[i + 1..end].iter().collect();
                result.push('_');
                result.push_str(&Self::escape_text(&inner));
                result.push('_');
                i = end + 1;
                continue;
            }

            // Markdown link: [text](url) → [text](url) (already MarkdownV2 compatible)
            if chars[i] == '['
                && let Some((link_text, url, end_pos)) = parse_markdown_link(&chars, i)
            {
                result.push('[');
                result.push_str(&Self::escape_text(&link_text));
                result.push_str("](");
                result.push_str(&url); // URLs don't get escaped
                result.push(')');
                i = end_pos;
                continue;
            }

            // Regular character — escape it
            if Self::ESCAPE_CHARS.contains(&chars[i]) {
                result.push('\\');
            }
            result.push(chars[i]);
            i += 1;
        }

        result
    }
}

impl ChannelFormatter for TelegramFormatter {
    fn platform(&self) -> &str {
        "telegram"
    }

    fn format(&self, content: &str) -> String {
        let cleaned = strip_internal_metadata(content);
        let cleaned = strip_bracket_citations(&cleaned);
        let cleaned = collapse_blank_lines(&cleaned);

        let mut out = Vec::new();
        let mut in_fence = false;
        let mut fence_lang = String::new();

        for line in cleaned.lines() {
            let trimmed = line.trim();

            // Code fence boundaries
            if trimmed.starts_with("```") {
                if in_fence {
                    // Close fence
                    out.push("```".to_string());
                    in_fence = false;
                    fence_lang.clear();
                } else {
                    // Open fence — extract language hint
                    fence_lang = trimmed.strip_prefix("```").unwrap_or("").trim().to_string();
                    if fence_lang.is_empty() {
                        out.push("```".to_string());
                    } else {
                        out.push(format!("```{fence_lang}"));
                    }
                    in_fence = true;
                }
                continue;
            }

            if in_fence {
                // Inside code block — no escaping, no conversion
                out.push(line.to_string());
            } else {
                out.push(Self::convert_line(line));
            }
        }

        // If we ended inside an unclosed fence, close it
        if in_fence {
            out.push("```".to_string());
        }

        out.join("\n").trim().to_string()
    }
}

// ── Discord ─────────────────────────────────────────────────────────────

/// Discord formatter — passes Markdown through with minimal cleanup.
///
/// Discord natively supports Markdown: **bold**, *italic*, `code`,
/// ```code blocks```, ~~strikethrough~~, > blockquotes, headers, links.
/// LLM output is already in the right format; we just strip metadata
/// and citations.
pub struct DiscordFormatter;

impl ChannelFormatter for DiscordFormatter {
    fn platform(&self) -> &str {
        "discord"
    }

    fn format(&self, content: &str) -> String {
        let cleaned = strip_internal_metadata(content);
        let cleaned = strip_bracket_citations(&cleaned);
        collapse_blank_lines(&cleaned).trim().to_string()
    }
}

// ── WhatsApp ────────────────────────────────────────────────────────────

/// WhatsApp formatter — converts Markdown to WhatsApp's formatting syntax.
///
/// WhatsApp supports: *bold*, _italic_, ~strikethrough~, ```monospace```,
/// and ``` ```code blocks``` ```. Markdown links become bare URLs.
pub struct WhatsAppFormatter;

impl WhatsAppFormatter {
    fn convert_line(line: &str) -> Cow<'_, str> {
        let trimmed = line.trim();

        // Headers → bold
        if let Some(rest) = trimmed
            .strip_prefix("### ")
            .or_else(|| trimmed.strip_prefix("## "))
            .or_else(|| trimmed.strip_prefix("# "))
        {
            return Cow::Owned(format!("*{}*", rest.trim()));
        }

        let mut result = line.to_string();

        // **bold** → *bold*
        while let Some(start) = result.find("**") {
            if let Some(end) = result[start + 2..].find("**") {
                let inner = result[start + 2..start + 2 + end].to_string();
                result = format!(
                    "{}*{}*{}",
                    &result[..start],
                    inner,
                    &result[start + 4 + end..]
                );
            } else {
                break;
            }
        }

        // __italic__ → _italic_ (already correct for WhatsApp)
        // `code` → ```code``` (WhatsApp inline monospace)
        let mut out = String::with_capacity(result.len());
        let chars: Vec<char> = result.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '`'
                && (i == 0 || chars[i - 1] != '`')
                && let Some(end) = find_closing(&chars, i + 1, '`')
            {
                let code: String = chars[i + 1..end].iter().collect();
                out.push_str("```");
                out.push_str(&code);
                out.push_str("```");
                i = end + 1;
                continue;
            }
            out.push(chars[i]);
            i += 1;
        }

        // [text](url) → url (bare links only)
        let final_result = strip_markdown_links(&out);
        Cow::Owned(final_result)
    }
}

impl ChannelFormatter for WhatsAppFormatter {
    fn platform(&self) -> &str {
        "whatsapp"
    }

    fn format(&self, content: &str) -> String {
        let cleaned = strip_internal_metadata(content);
        let cleaned = strip_bracket_citations(&cleaned);
        let cleaned = collapse_blank_lines(&cleaned);

        let mut out = Vec::new();
        let mut in_fence = false;

        for line in cleaned.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                if in_fence {
                    out.push("```".to_string());
                    in_fence = false;
                } else {
                    out.push("```".to_string());
                    in_fence = true;
                }
                continue;
            }
            if in_fence {
                out.push(line.to_string());
            } else {
                out.push(Self::convert_line(line).into_owned());
            }
        }

        if in_fence {
            out.push("```".to_string());
        }

        out.join("\n").trim().to_string()
    }
}

// ── Signal ──────────────────────────────────────────────────────────────

/// Signal formatter — plain text only.
///
/// Signal Protocol has no formatting support. All Markdown syntax is stripped
/// to produce clean, readable plain text.
pub struct SignalFormatter;

impl ChannelFormatter for SignalFormatter {
    fn platform(&self) -> &str {
        "signal"
    }

    fn format(&self, content: &str) -> String {
        let cleaned = strip_internal_metadata(content);
        let cleaned = strip_bracket_citations(&cleaned);

        let mut out = Vec::new();
        let mut in_fence = false;

        for line in cleaned.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                in_fence = !in_fence;
                continue; // drop fence markers
            }
            if in_fence {
                // Preserve code content as-is (indented for readability)
                out.push(format!("  {line}"));
            } else {
                // Strip all Markdown formatting
                let mut plain = line.trim_start_matches('#').trim_start().to_string();
                plain = plain.replace("**", "").replace("__", "").replace('`', "");
                plain = strip_markdown_links(&plain);
                out.push(plain);
            }
        }

        collapse_blank_lines(&out.join("\n")).trim().to_string()
    }
}

// ── Web ─────────────────────────────────────────────────────────────────

/// Web/WebSocket formatter — preserves Markdown for client-side rendering.
///
/// The web dashboard renders Markdown client-side via `renderSafeMarkdown()`.
/// This formatter only strips internal metadata; all Markdown passes through
/// intact for the JavaScript renderer.
pub struct WebFormatter;

impl ChannelFormatter for WebFormatter {
    fn platform(&self) -> &str {
        "web"
    }

    fn format(&self, content: &str) -> String {
        let cleaned = strip_internal_metadata(content);
        collapse_blank_lines(&cleaned).trim().to_string()
    }
}

// ── Email ───────────────────────────────────────────────────────────────

/// Email formatter — preserves Markdown for rich-text email rendering.
///
/// Email clients that support HTML can render Markdown; plain-text clients
/// degrade gracefully since Markdown is human-readable. This formatter
/// strips metadata and citations but preserves all formatting structure.
pub struct EmailFormatter;

impl ChannelFormatter for EmailFormatter {
    fn platform(&self) -> &str {
        "email"
    }

    fn format(&self, content: &str) -> String {
        let cleaned = strip_internal_metadata(content);
        let cleaned = strip_bracket_citations(&cleaned);
        collapse_blank_lines(&cleaned).trim().to_string()
    }
}

// ── Registry ────────────────────────────────────────────────────────────

/// Look up the appropriate formatter for a platform name.
///
/// Returns the channel-specific formatter, falling back to [`WebFormatter`]
/// for unknown platforms (safe because web-style Markdown is the most
/// permissive format).
pub fn formatter_for(platform: &str) -> &'static dyn ChannelFormatter {
    match platform.to_ascii_lowercase().as_str() {
        "telegram" => &TelegramFormatter,
        "discord" => &DiscordFormatter,
        "whatsapp" => &WhatsAppFormatter,
        "signal" => &SignalFormatter,
        "email" => &EmailFormatter,
        _ => &WebFormatter,
    }
}

// ── Inline parsing helpers ──────────────────────────────────────────────

/// Find the position of a closing delimiter character, skipping escaped chars.
fn find_closing(chars: &[char], start: usize, delim: char) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == delim {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Find closing double delimiter (e.g., ** or __).
fn find_double_closing(chars: &[char], start: usize, delim: char) -> Option<usize> {
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == delim && chars[i + 1] == delim {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Find closing single delimiter that is NOT immediately followed by itself.
fn find_closing_not_doubled(chars: &[char], start: usize, delim: char) -> Option<usize> {
    let mut i = start;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == delim {
            if i + 1 < chars.len() && chars[i + 1] == delim {
                i += 2; // skip doubled
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Parse a Markdown link `[text](url)` starting at position `start`.
/// Returns `(link_text, url, end_position)` or None.
fn parse_markdown_link(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    if start >= chars.len() || chars[start] != '[' {
        return None;
    }
    let mut i = start + 1;
    let mut depth = 1;
    // Find closing ]
    while i < chars.len() && depth > 0 {
        if chars[i] == '[' {
            depth += 1;
        }
        if chars[i] == ']' {
            depth -= 1;
        }
        if depth > 0 {
            i += 1;
        }
    }
    if depth != 0 || i >= chars.len() {
        return None;
    }
    let text: String = chars[start + 1..i].iter().collect();
    i += 1; // skip ]
    if i >= chars.len() || chars[i] != '(' {
        return None;
    }
    i += 1; // skip (
    let url_start = i;
    let mut paren_depth = 1;
    while i < chars.len() && paren_depth > 0 {
        if chars[i] == '(' {
            paren_depth += 1;
        }
        if chars[i] == ')' {
            paren_depth -= 1;
        }
        if paren_depth > 0 {
            i += 1;
        }
    }
    if paren_depth != 0 {
        return None;
    }
    let url: String = chars[url_start..i].iter().collect();
    Some((text, url, i + 1))
}

/// Replace `[text](url)` with just `url` for channels that only support bare links.
fn strip_markdown_links(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '['
            && let Some((_link_text, url, end_pos)) = parse_markdown_link(&chars, i)
        {
            out.push_str(&url);
            i = end_pos;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- Shared utility tests --

    #[test]
    fn strip_metadata_removes_delegation_lines() {
        let input = "Hello\n[Delegated to sub-agent]\nWorld\n[Tool call: search]\nDone";
        let result = strip_internal_metadata(input);
        assert_eq!(result, "Hello\nWorld\nDone");
    }

    #[test]
    fn strip_citations_removes_numeric_brackets() {
        assert_eq!(
            strip_bracket_citations("See [1] and [23] here"),
            "See  and  here"
        );
        assert_eq!(
            strip_bracket_citations("Keep [text] and [a1]"),
            "Keep [text] and [a1]"
        );
    }

    #[test]
    fn collapse_blanks() {
        let input = "a\n\n\n\nb\n\nc";
        assert_eq!(collapse_blank_lines(input), "a\n\nb\n\nc");
    }

    // -- Telegram tests --

    #[test]
    fn telegram_escapes_special_chars() {
        let result = TelegramFormatter.format("Hello. World! Test-1 (ok)");
        assert!(result.contains("Hello\\."));
        assert!(result.contains("World\\!"));
        assert!(result.contains("Test\\-1"));
        assert!(result.contains("\\(ok\\)"));
    }

    #[test]
    fn telegram_headers_become_bold() {
        let result = TelegramFormatter.format("# Main Title\n## Section\n### Sub");
        assert!(result.contains("*Main Title*"));
        assert!(result.contains("*Section*"));
        assert!(result.contains("*Sub*"));
    }

    #[test]
    fn telegram_preserves_code_blocks() {
        let input = "Before\n```rust\nfn main() { }\n```\nAfter";
        let result = TelegramFormatter.format(input);
        assert!(result.contains("```rust"));
        assert!(result.contains("fn main() { }"));
        assert!(result.contains("```"));
    }

    #[test]
    fn telegram_bold_conversion() {
        let result = TelegramFormatter.format("This is **bold** text");
        assert!(result.contains("*bold*"));
    }

    #[test]
    fn telegram_inline_code() {
        let result = TelegramFormatter.format("Use `npm install` here");
        assert!(result.contains("`npm install`"));
    }

    #[test]
    fn telegram_strips_metadata() {
        let input = "Hello\n[Delegated to worker]\nWorld";
        let result = TelegramFormatter.format(input);
        assert!(!result.contains("Delegated"));
        assert!(result.contains("Hello"));
        assert!(result.contains("World"));
    }

    #[test]
    fn telegram_strips_citations() {
        let result = TelegramFormatter.format("According to [1] the data [23] shows");
        assert!(!result.contains("[1]"));
        assert!(!result.contains("[23]"));
    }

    #[test]
    fn telegram_strikethrough_conversion() {
        let result = TelegramFormatter.format("This is ~~deleted~~ text");
        assert!(
            result.contains("~deleted~"),
            "strikethrough should be ~deleted~, got: {result}"
        );
        // Should NOT contain the double tilde
        assert!(!result.contains("~~"));
    }

    #[test]
    fn telegram_single_tilde_strikethrough() {
        // LLMs sometimes output Telegram-native ~text~ instead of Markdown ~~text~~
        let result = TelegramFormatter.format("This is ~deleted~ text");
        assert!(
            result.contains("~deleted~"),
            "single-tilde strikethrough should be preserved as ~deleted~, got: {result}"
        );
        // The tildes must NOT be escaped (would show as literal \~ in Telegram)
        assert!(
            !result.contains("\\~"),
            "tildes should not be escaped in strikethrough, got: {result}"
        );
    }

    #[test]
    fn telegram_single_underscore_italic() {
        let result = TelegramFormatter.format("This is _italic_ text");
        assert!(
            result.contains("_italic_"),
            "single underscore italic should be preserved, got: {result}"
        );
        // The underscore should NOT be escaped
        assert!(
            !result.contains("\\_italic\\_"),
            "underscores should not be escaped for italic, got: {result}"
        );
    }

    #[test]
    fn telegram_blockquote() {
        let result = TelegramFormatter.format("Normal line\n> This is a quote\nAfter");
        assert!(
            result.contains(">This is a quote"),
            "blockquote should start with >, got: {result}"
        );
        assert!(
            !result.contains("\\>"),
            "> should not be escaped in blockquotes, got: {result}"
        );
    }

    #[test]
    fn telegram_empty_blockquote() {
        let result = TelegramFormatter.format("Before\n>\nAfter");
        assert!(result.contains("\n>\n") || result.contains("\n>"));
    }

    #[test]
    fn telegram_combined_formatting() {
        let result = TelegramFormatter.format("**Bold** and _italic_ and ~~struck~~ and > quoted");
        assert!(result.contains("*Bold*"), "bold failed: {result}");
        assert!(result.contains("_italic_"), "italic failed: {result}");
        assert!(
            result.contains("~struck~"),
            "strikethrough failed: {result}"
        );
    }

    // -- Discord tests --

    #[test]
    fn discord_preserves_markdown() {
        let input = "# Header\n**bold** and *italic*\n```rust\ncode\n```";
        let result = DiscordFormatter.format(input);
        assert!(result.contains("# Header"));
        assert!(result.contains("**bold**"));
        assert!(result.contains("*italic*"));
        assert!(result.contains("```rust"));
    }

    #[test]
    fn discord_strips_metadata() {
        let input = "Hello\n[Tool call: search]\nWorld";
        let result = DiscordFormatter.format(input);
        assert!(!result.contains("Tool call"));
    }

    // -- WhatsApp tests --

    #[test]
    fn whatsapp_headers_become_bold() {
        let result = WhatsAppFormatter.format("# Title\nBody text");
        assert!(result.contains("*Title*"));
        assert!(result.contains("Body text"));
    }

    #[test]
    fn whatsapp_bold_conversion() {
        let result = WhatsAppFormatter.format("This is **bold** here");
        assert!(result.contains("*bold*"));
        assert!(!result.contains("**"));
    }

    #[test]
    fn whatsapp_inline_code_triple_backtick() {
        let result = WhatsAppFormatter.format("Use `npm install` here");
        assert!(result.contains("```npm install```"));
    }

    #[test]
    fn whatsapp_links_become_bare_urls() {
        let result = WhatsAppFormatter.format("See [docs](https://example.com) here");
        assert!(result.contains("https://example.com"));
        assert!(!result.contains("[docs]"));
    }

    // -- Signal tests --

    #[test]
    fn signal_strips_all_formatting() {
        let input = "# Title\n**bold** and *italic*\n`code`";
        let result = SignalFormatter.format(input);
        assert!(!result.contains('#'));
        assert!(!result.contains("**"));
        assert!(!result.contains('`'));
        assert!(result.contains("Title"));
        assert!(result.contains("bold"));
        assert!(result.contains("code"));
    }

    #[test]
    fn signal_indents_code_blocks() {
        let input = "Before\n```\nfn main()\n```\nAfter";
        let result = SignalFormatter.format(input);
        assert!(result.contains("  fn main()"));
        assert!(!result.contains("```"));
    }

    #[test]
    fn signal_links_become_bare_urls() {
        let result = SignalFormatter.format("See [docs](https://example.com)");
        assert!(result.contains("https://example.com"));
        assert!(!result.contains("[docs]"));
    }

    // -- Web tests --

    #[test]
    fn web_preserves_everything() {
        let input = "# Header\n**bold**\n```code```\n[link](url)";
        let result = WebFormatter.format(input);
        assert!(result.contains("# Header"));
        assert!(result.contains("**bold**"));
        assert!(result.contains("[link](url)"));
    }

    #[test]
    fn web_strips_metadata_only() {
        let input = "Hello\n[Delegated to sub]\nWorld";
        let result = WebFormatter.format(input);
        assert!(!result.contains("Delegated"));
        assert!(result.contains("Hello"));
    }

    // -- Email tests --

    #[test]
    fn email_preserves_markdown_strips_citations() {
        let input = "# Report\nSee [1] for details.\n**Important**";
        let result = EmailFormatter.format(input);
        assert!(result.contains("# Report"));
        assert!(result.contains("**Important**"));
        assert!(!result.contains("[1]"));
    }

    // -- Registry tests --

    #[test]
    fn formatter_for_known_platforms() {
        assert_eq!(formatter_for("telegram").platform(), "telegram");
        assert_eq!(formatter_for("Telegram").platform(), "telegram");
        assert_eq!(formatter_for("discord").platform(), "discord");
        assert_eq!(formatter_for("whatsapp").platform(), "whatsapp");
        assert_eq!(formatter_for("signal").platform(), "signal");
        assert_eq!(formatter_for("email").platform(), "email");
        assert_eq!(formatter_for("web").platform(), "web");
        assert_eq!(formatter_for("websocket").platform(), "web");
    }

    #[test]
    fn formatter_for_unknown_falls_back_to_web() {
        assert_eq!(formatter_for("sms").platform(), "web");
        assert_eq!(formatter_for("carrier_pigeon").platform(), "web");
    }

    // -- Edge cases --

    #[test]
    fn empty_input() {
        assert_eq!(TelegramFormatter.format(""), "");
        assert_eq!(DiscordFormatter.format(""), "");
        assert_eq!(SignalFormatter.format(""), "");
        assert_eq!(WhatsAppFormatter.format(""), "");
        assert_eq!(WebFormatter.format(""), "");
    }

    #[test]
    fn unclosed_code_fence() {
        let input = "Start\n```\ncode here\nmore code";
        let result = TelegramFormatter.format(input);
        assert!(result.contains("```")); // should auto-close
        assert!(result.contains("code here"));
    }

    #[test]
    fn telegram_link_conversion() {
        let result = TelegramFormatter.format("See [the docs](https://example.com) here");
        assert!(result.contains("[the docs](https://example.com)"));
    }

    #[test]
    fn strip_markdown_links_preserves_non_links() {
        assert_eq!(strip_markdown_links("plain text"), "plain text");
        assert_eq!(strip_markdown_links("[just brackets]"), "[just brackets]");
    }

    #[test]
    fn strip_markdown_links_extracts_urls() {
        assert_eq!(
            strip_markdown_links("See [docs](https://example.com) here"),
            "See https://example.com here"
        );
    }

    #[test]
    fn parse_link_nested_parens() {
        let chars: Vec<char> = "[text](http://x.com/p(1))".chars().collect();
        let result = parse_markdown_link(&chars, 0);
        assert!(result.is_some());
        let (text, url, _) = result.unwrap();
        assert_eq!(text, "text");
        assert_eq!(url, "http://x.com/p(1)");
    }

    #[test]
    fn telegram_mixed_formatting() {
        let input = "# Status Report\n\nSystem is **online** and running `v2.1`.\n\nSee [dashboard](https://dash.example.com) for details.\n\n```\nuptime: 99.9%\n```";
        let result = TelegramFormatter.format(input);
        // Header becomes bold
        assert!(result.contains("*Status Report*"));
        // Bold preserved
        assert!(result.contains("*online*"));
        // Code preserved
        assert!(result.contains("`v2.1`"));
        // Code block preserved
        assert!(result.contains("uptime: 99.9%"));
    }
}
