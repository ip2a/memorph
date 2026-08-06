//! Session content sanitizer: strips tokenizer-hostile control characters
//! from canonical session text fields before a session is written to a target
//! provider.
//!
//! Scope is deliberately narrow. The only characters removed are the C0
//! control range (`U+0000`–`U+001F`) minus tab/newline/carriage-return, plus
//! DEL (`U+007F`) — bytes that provider backends routinely reject with a
//! `tokenization failed` 400, most notoriously the NUL byte. Everything else
//! (special-token-like strings, private-use Unicode) is intentionally left
//! alone: those have legitimate uses in developer-facing text and stripping
//! them silently would both corrupt content and violate memorph's fidelity
//! contract.
//!
//! Sanitization runs at the transfer boundary (`switch`/`import`) on the
//! session about to be written, never on the stored canonical session, so the
//! high-fidelity source stays intact and every cleaning pass is observable
//! via the returned [`SanitizeReport`] (logged by the caller).

use crate::session::{Block, Session};

/// Aggregate counters from a sanitize pass, for observability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SanitizeReport {
    /// Total control characters removed across all text fields.
    pub control_chars_removed: usize,
    /// Number of block text fields that had at least one character removed.
    pub fields_modified: usize,
}

impl SanitizeReport {
    /// True when the pass removed nothing.
    pub fn is_clean(&self) -> bool {
        self.control_chars_removed == 0
    }
}

/// Remove tokenizer-hostile control characters from `text` in place and
/// return the number of characters removed.
///
/// Strips the C0 range (`U+0000`–`U+001F`) except `\t` `\n` `\r`, plus DEL
/// (`U+007F`). `String::retain` keeps the common clean-text fast path as a
/// single pass with no reallocation.
pub fn sanitize_text(text: &mut String) -> usize {
    let mut removed = 0;
    text.retain(|c| {
        if is_control_char(c) {
            removed += 1;
            false
        } else {
            true
        }
    });
    removed
}

/// Whether `c` is a control character targeted by sanitization.
fn is_control_char(c: char) -> bool {
    // C0 range minus \t (0x09), \n (0x0A), \r (0x0D), plus DEL (0x7F).
    matches!(
        c,
        '\0'..='\u{08}' | '\u{0B}' | '\u{0C}' | '\u{0E}'..='\u{1F}' | '\u{7F}'
    )
}

/// Sanitize every free-text field of a canonical session in place, returning
/// aggregate counters.
///
/// Only fields that become conversational text in the target provider are
/// touched. Structured payloads (`ToolCall::input`, `Compressed::raw`,
/// `Other::raw`), identifiers (`tool_call_id`, paths, MIME types), and image
/// data are skipped — mutating those would break cross-message pairing or
/// corrupt fidelity-preservation blobs.
pub fn sanitize_session(session: &mut Session) -> SanitizeReport {
    let mut report = SanitizeReport::default();
    for event in &mut session.events {
        for block in &mut event.blocks {
            sanitize_block(block, &mut report);
        }
    }
    report
}

fn sanitize_block(block: &mut Block, report: &mut SanitizeReport) {
    let mut modified = false;
    match block {
        Block::Text { text } => {
            modified |= sanitize_field(text, report);
        }
        Block::Thinking { text, .. } => {
            modified |= sanitize_field(text, report);
        }
        Block::ToolResult { content, .. } => {
            modified |= sanitize_field(content, report);
        }
        Block::Patch {
            summary,
            diff_text,
            files,
            ..
        } => {
            if let Some(text) = summary {
                modified |= sanitize_field(text, report);
            }
            if let Some(text) = diff_text {
                modified |= sanitize_field(text, report);
            }
            for file in files {
                modified |= sanitize_field(file, report);
            }
        }
        Block::Command { command, argv, .. } => {
            modified |= sanitize_field(command, report);
            for arg in argv {
                modified |= sanitize_field(arg, report);
            }
        }
        Block::CommandResult {
            stdout,
            stderr,
            ..
        } => {
            if let Some(text) = stdout {
                modified |= sanitize_field(text, report);
            }
            if let Some(text) = stderr {
                modified |= sanitize_field(text, report);
            }
        }
        Block::File { content, .. } => {
            if let Some(text) = content {
                modified |= sanitize_field(text, report);
            }
        }
        // ToolCall.input (structured JSON), Image (binary/path), Compressed
        // and Other (fidelity-preservation raw payloads): not free text.
        Block::ToolCall { .. } | Block::Image { .. } | Block::Compressed { .. } | Block::Other { .. } => {}
    }
    if modified {
        report.fields_modified += 1;
    }
}

fn sanitize_field(text: &mut String, report: &mut SanitizeReport) -> bool {
    let removed = sanitize_text(text);
    report.control_chars_removed += removed;
    removed > 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{Event, EventKind, Identity, Links, Metadata, Role};

    #[test]
    fn strips_nul_and_c0_but_keeps_tab_newline_cr() {
        let mut text = "a\x00b\x01c\x02d\x1Fe".to_string();
        assert_eq!(sanitize_text(&mut text), 4);
        assert_eq!(text, "abcde");

        let mut text = "line1\nline2\tcol\rcr".to_string();
        assert_eq!(sanitize_text(&mut text), 0);
        assert_eq!(text, "line1\nline2\tcol\rcr");
    }

    #[test]
    fn strips_vertical_tab_form_feed_and_del() {
        // \x0B (VT) and \x0C (FF) sit between \n and \r and are control chars.
        let mut text = "a\x0Bb\x0Cc\x7Fd".to_string();
        assert_eq!(sanitize_text(&mut text), 3);
        assert_eq!(text, "abcd");
    }

    #[test]
    fn preserves_multibyte_utf8_and_emoji() {
        let mut text = "你好👋\x00世界🦀".to_string();
        assert_eq!(sanitize_text(&mut text), 1);
        assert_eq!(text, "你好👋世界🦀");
    }

    #[test]
    fn clean_text_is_unchanged_and_reports_zero() {
        let mut text = "plain ascii and 你好".to_string();
        assert_eq!(sanitize_text(&mut text), 0);
        assert_eq!(text, "plain ascii and 你好");
    }

    #[test]
    fn empty_string_is_noop() {
        let mut text = String::new();
        assert_eq!(sanitize_text(&mut text), 0);
        assert!(text.is_empty());
    }

    fn text_block(text: &str) -> Block {
        Block::Text {
            text: text.to_string(),
        }
    }

    fn text_event(id: &str, blocks: Vec<Block>) -> Event {
        Event {
            id: id.to_string(),
            kind: EventKind::Message,
            role: Role::User,
            timestamp: chrono::Utc::now(),
            links: Links::default(),
            blocks,
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: Metadata {
                model: None,
                usage: None,
            },
        }
    }

    #[test]
    fn sanitize_session_strips_dirty_text_blocks_and_counts() {
        let mut session = Session {
            lineage: Vec::new(),
            schema: Default::default(),
            identity: Identity {
                id: "s1".to_string(),
                title: None,
            },
            context: Default::default(),
            events: vec![
                text_event("e1", vec![text_block("clean text")]),
                text_event("e2", vec![text_block("bad\x00word \x01more")]),
            ],
            extensions: Default::default(),
        };

        let report = sanitize_session(&mut session);
        assert_eq!(report.control_chars_removed, 2);
        assert_eq!(report.fields_modified, 1);
        assert!(!report.is_clean());

        match &session.events[1].blocks[0] {
            Block::Text { text } => assert_eq!(text, "badword more"),
            other => panic!("expected text block, got {other:?}"),
        }
    }

    #[test]
    fn sanitize_session_leaves_clean_session_untouched() {
        let mut session = Session {
            lineage: Vec::new(),
            schema: Default::default(),
            identity: Identity {
                id: "s1".to_string(),
                title: None,
            },
            context: Default::default(),
            events: vec![text_event("e1", vec![text_block("no problems here")])],
            extensions: Default::default(),
        };

        let report = sanitize_session(&mut session);
        assert!(report.is_clean());
        assert_eq!(report.control_chars_removed, 0);
        assert_eq!(report.fields_modified, 0);
    }

    #[test]
    fn sanitize_block_skips_structured_payloads() {
        // ToolCall.input JSON must pass through untouched.
        let mut block = Block::ToolCall {
            tool_call_id: "c1".to_string(),
            name: "run".to_string(),
            input: Some(serde_json::json!({ "cmd": "echo\x00hi" })),
        };
        let mut report = SanitizeReport::default();
        sanitize_block(&mut block, &mut report);
        assert!(report.is_clean(), "ToolCall input must not be sanitized");
        match &block {
            Block::ToolCall { input, .. } => {
                assert_eq!(
                    input.as_ref().unwrap()["cmd"],
                    "echo\x00hi",
                    "NUL inside JSON input is preserved"
                );
            }
            _ => panic!("expected tool call"),
        }

        // Other.raw fidelity blob must pass through untouched.
        let mut block = Block::Other {
            raw: serde_json::json!({ "x": "a\x00b" }),
        };
        let mut report = SanitizeReport::default();
        sanitize_block(&mut block, &mut report);
        assert!(report.is_clean(), "Other raw payload must not be sanitized");
    }
}
