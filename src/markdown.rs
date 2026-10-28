//! GitHub-flavoured Markdown as terminal lines: headings, emphasis, lists, task lists,
//! quotes, code, links and tables. HTML tags and comments are dropped; their text stays.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

/// `markdown` rendered as lines, each starting with `indent`.
pub fn lines(markdown: &str, indent: &str) -> Vec<Line<'static>> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM;
    let mut out = Renderer {
        indent: indent.to_owned(),
        ..Renderer::default()
    };
    for event in Parser::new_ext(markdown, options) {
        out.event(event);
    }
    out.flush();
    while out.lines.last().is_some_and(|line| line.width() == 0) {
        out.lines.pop();
    }
    out.lines
}

#[derive(Default)]
struct Renderer {
    indent: String,
    lines: Vec<Line<'static>>,
    spans: Vec<Span<'static>>,
    styles: Vec<Style>,
    /// One entry per open list: the next number of an ordered list.
    lists: Vec<Option<u64>>,
    /// The marker the current list item's first line still needs.
    marker: Option<String>,
    quotes: usize,
    code: bool,
    links: Vec<String>,
    in_comment: bool,
}

impl Renderer {
    fn style(&self) -> Style {
        self.styles
            .iter()
            .fold(Style::default(), |all, style| all.patch(*style))
    }

    fn push(&mut self, text: impl Into<String>, style: Style) {
        let text = text.into();
        if !text.is_empty() {
            self.spans.push(Span::styled(text, style));
        }
    }

    fn prefix(&self) -> String {
        let depth = self.lists.len().saturating_sub(1);
        format!(
            "{}{}{}",
            self.indent,
            "│ ".repeat(self.quotes),
            "  ".repeat(depth)
        )
    }

    /// Ends the current line (if it has anything).
    fn flush(&mut self) {
        if self.spans.is_empty() && self.marker.is_none() {
            return;
        }
        let mut spans = vec![Span::raw(self.prefix())];
        if let Some(marker) = self.marker.take() {
            spans.push(Span::styled(
                marker,
                Style::default().fg(crate::theme::p().sky),
            ));
        }
        spans.append(&mut self.spans);
        self.lines.push(Line::from(spans));
    }

    fn blank(&mut self) {
        self.flush();
        if self.lines.last().is_some_and(|line| line.width() > 0) {
            self.lines.push(Line::raw(""));
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) if self.code => {
                for line in text.lines() {
                    let code = Style::default().fg(crate::theme::p().yellow);
                    let prefix = format!("{}    ", self.prefix());
                    self.lines.push(Line::from(vec![
                        Span::raw(prefix),
                        Span::styled(line.replace('\t', "    "), code),
                    ]));
                }
            }
            Event::Text(text) => {
                let style = self.style();
                self.push(text.to_string(), style);
            }
            Event::Code(code) => self.push(
                code.to_string(),
                Style::default().fg(crate::theme::p().yellow),
            ),
            Event::Html(html) | Event::InlineHtml(html) => {
                let text = self.strip_html(&html);
                let style = self.style();
                for (index, part) in text.split('\n').enumerate() {
                    if index > 0 {
                        self.flush();
                    }
                    self.push(part.trim().to_owned(), style);
                }
            }
            Event::SoftBreak => self.push(" ", Style::default()),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.blank();
                self.lines
                    .push(Line::raw(format!("{}────────", self.prefix())));
                self.blank();
            }
            Event::TaskListMarker(done) => {
                let mark = if done { "☑ " } else { "☐ " };
                self.marker = Some(mark.into());
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.blank();
                let mut style = Style::default().add_modifier(Modifier::BOLD);
                if level == HeadingLevel::H1 {
                    style = style.add_modifier(Modifier::UNDERLINED);
                }
                self.styles.push(style);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quotes += 1;
            }
            Tag::CodeBlock(kind) => {
                self.blank();
                self.code = true;
                if let CodeBlockKind::Fenced(language) = kind
                    && !language.is_empty()
                {
                    let dim = Style::default().add_modifier(Modifier::DIM);
                    self.lines.push(Line::from(vec![
                        Span::raw(format!("{}    ", self.prefix())),
                        Span::styled(language.to_string(), dim),
                    ]));
                }
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let marker = match self.lists.last_mut() {
                    Some(Some(number)) => {
                        *number += 1;
                        format!("{}. ", *number - 1)
                    }
                    _ => "• ".into(),
                };
                self.marker = Some(marker);
            }
            Tag::Emphasis => self
                .styles
                .push(Style::default().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self
                .styles
                .push(Style::default().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => {
                let crossed = Style::default().add_modifier(Modifier::CROSSED_OUT);
                self.styles.push(crossed);
            }
            Tag::Link { dest_url, .. } => {
                self.links.push(dest_url.to_string());
                self.styles
                    .push(Style::default().add_modifier(Modifier::UNDERLINED));
            }
            Tag::Image { dest_url, .. } => {
                self.links.push(dest_url.to_string());
                self.push("[image: ", Style::default().add_modifier(Modifier::DIM));
                self.styles
                    .push(Style::default().add_modifier(Modifier::DIM));
            }
            Tag::TableHead => self
                .styles
                .push(Style::default().add_modifier(Modifier::BOLD)),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.styles.pop();
                self.blank();
            }
            TagEnd::Paragraph => {
                if self.lists.is_empty() {
                    self.blank();
                } else {
                    self.flush();
                }
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quotes = self.quotes.saturating_sub(1);
                self.blank();
            }
            TagEnd::CodeBlock => {
                self.code = false;
                self.blank();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => self.flush(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::Link => {
                self.styles.pop();
                if let Some(url) = self.links.pop() {
                    let text: String = self.spans.iter().map(|s| s.content.as_ref()).collect();
                    if !text.ends_with(url.as_str()) && !url.starts_with('#') {
                        self.push(
                            format!(" ({url})"),
                            Style::default().add_modifier(Modifier::DIM),
                        );
                    }
                }
            }
            TagEnd::Image => {
                self.styles.pop();
                self.links.pop();
                self.push("]", Style::default().add_modifier(Modifier::DIM));
            }
            TagEnd::TableCell => self.push(" │ ", Style::default().add_modifier(Modifier::DIM)),
            TagEnd::TableHead => {
                self.styles.pop();
                self.flush();
            }
            TagEnd::TableRow => self.flush(),
            TagEnd::Table => self.blank(),
            _ => {}
        }
    }

    /// The text of an HTML fragment: comments and tags removed, `<br>` as a new line.
    fn strip_html(&mut self, html: &str) -> String {
        let mut text = String::new();
        let mut rest = html;
        while !rest.is_empty() {
            if self.in_comment {
                match rest.find("-->") {
                    Some(end) => {
                        self.in_comment = false;
                        rest = &rest[end + 3..];
                    }
                    None => return text,
                }
            } else if let Some(after) = rest.strip_prefix("<!--") {
                self.in_comment = true;
                rest = after;
            } else if rest.starts_with('<') {
                let end = rest.find('>').map_or(rest.len(), |end| end + 1);
                let tag = rest[..end].to_ascii_lowercase();
                if tag.starts_with("<br") || tag.starts_with("</p") || tag.starts_with("</div") {
                    text.push('\n');
                }
                rest = &rest[end..];
            } else {
                let end = rest.find('<').unwrap_or(rest.len());
                text.push_str(&rest[..end].replace("&nbsp;", " ").replace("&amp;", "&"));
                rest = &rest[end..];
            }
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(markdown: &str) -> Vec<String> {
        lines(markdown, "").iter().map(|l| l.to_string()).collect()
    }

    #[test]
    fn headings_paragraphs_and_emphasis() {
        let out = lines("# Title\n\nSome **bold** and *it*.", "");
        assert_eq!(out[0].to_string(), "Title");
        assert!(
            out[0]
                .spans
                .iter()
                .any(|s| s.style.add_modifier.contains(Modifier::BOLD))
        );
        assert_eq!(out[2].to_string(), "Some bold and it.");
    }

    #[test]
    fn lists_task_lists_and_nesting() {
        assert_eq!(
            text("- one\n- two\n  - inner\n\n1. first\n2. second\n\n- [x] done\n- [ ] todo"),
            [
                "• one",
                "• two",
                "  • inner",
                "",
                "1. first",
                "2. second",
                "",
                "☑ done",
                "☐ todo"
            ]
        );
    }

    #[test]
    fn code_quotes_links_and_rules() {
        let out = text(
            "Run `cargo test`:\n\n```rust\nfn main() {}\n```\n\n> quoted\n\n[docs](https://x.y)\n\n---",
        );
        assert_eq!(out[0], "Run cargo test:");
        assert!(out.contains(&"    rust".to_string()));
        assert!(out.contains(&"    fn main() {}".to_string()));
        assert!(out.contains(&"│ quoted".to_string()));
        assert!(out.contains(&"docs (https://x.y)".to_string()));
        assert!(out.contains(&"────────".to_string()));
    }

    #[test]
    fn html_comments_and_tags_are_dropped_but_their_text_stays() {
        let out = text(
            "<!-- generated:v1 -->\n<details>\n<summary>Walkthrough</summary>\n\nThe change.\n</details>",
        );
        assert!(out.iter().all(|line| !line.contains('<')), "{out:?}");
        assert!(out.contains(&"Walkthrough".to_string()), "{out:?}");
        assert!(out.contains(&"The change.".to_string()), "{out:?}");
    }

    #[test]
    fn tables_and_indent() {
        let out = text("| a | b |\n|---|---|\n| 1 | 2 |");
        assert_eq!(out[0], "a │ b │ ");
        assert_eq!(out[1], "1 │ 2 │ ");
        assert_eq!(lines("hi", "  ")[0].to_string(), "  hi");
    }
}
