//! Patches for staging part of a file: one hunk, or one changed line, cut from the file's
//! unified diff and given to `git apply --cached`.
//!
//! Staging applies a patch to the index (the diff's old side); unstaging applies the staged
//! diff in reverse (the index is its new side). A one-line patch keeps the chosen line and
//! rewrites the others as `git add -p` does: lines missing from the side the patch applies
//! to are dropped, and lines present there become context.

/// The file headers of a one-file diff and the line ranges of its hunks.
#[derive(Debug, PartialEq, Eq)]
pub struct FileDiff<'a> {
    lines: Vec<&'a str>,
    /// Index of the first `@@` line.
    first_hunk: usize,
    /// `(start, end)` line ranges, each starting at its `@@` header.
    hunks: Vec<(usize, usize)>,
}

impl<'a> FileDiff<'a> {
    /// Splits a one-file diff (as `git::diff_of` returns it); `None` without hunks, as for
    /// a binary file or a mode change.
    pub fn parse(diff: &'a str) -> Option<Self> {
        // Lines keep their endings, so a CRLF file's `\r` survives into the patch.
        let lines: Vec<&str> = diff.split_inclusive('\n').collect();
        // A conflict's combined diff, a rename or a copy is staged whole: a partial
        // patch cannot express it (reversed, a rename header would undo the rename).
        let whole_only = |line: &&str| {
            line.starts_with("diff --cc ")
                || line.starts_with("rename from ")
                || line.starts_with("copy from ")
        };
        let starts: Vec<usize> = (0..lines.len())
            .filter(|&i| lines[i].starts_with("@@"))
            .collect();
        let first_hunk = *starts.first()?;
        if lines[..first_hunk].iter().any(whole_only) {
            return None;
        }
        let hunks = starts
            .iter()
            .enumerate()
            .map(|(n, &start)| (start, starts.get(n + 1).copied().unwrap_or(lines.len())))
            .collect();
        Some(Self {
            lines,
            first_hunk,
            hunks,
        })
    }

    /// Whether diff line `index` is an added or removed line (what a cursor may rest on).
    pub fn is_change(&self, index: usize) -> bool {
        index >= self.first_hunk
            && self
                .lines
                .get(index)
                .is_some_and(|line| line.starts_with('+') || line.starts_with('-'))
    }

    /// The hunk containing diff line `index`.
    fn hunk_of(&self, index: usize) -> Option<(usize, usize)> {
        self.hunks
            .iter()
            .copied()
            .find(|&(start, end)| (start..end).contains(&index))
    }

    /// The file headers, without a mode change: staging part of the text should not
    /// stage the mode too.
    fn header(&self) -> String {
        let mut out = String::new();
        for line in &self.lines[..self.first_hunk] {
            if !line.starts_with("old mode ") && !line.starts_with("new mode ") {
                push(&mut out, line);
            }
        }
        out
    }

    /// A patch of the whole hunk containing diff line `index`.
    pub fn hunk_patch(&self, index: usize) -> Option<String> {
        let (start, end) = self.hunk_of(index)?;
        let mut out = self.header();
        for line in &self.lines[start..end] {
            push(&mut out, line);
        }
        Some(out)
    }

    /// A patch of the single changed line `index`. `reverse` builds it for unstaging,
    /// where it is applied with `--reverse` to the index (the diff's new side).
    pub fn line_patch(&self, index: usize, reverse: bool) -> Option<String> {
        if !self.is_change(index) {
            return None;
        }
        let (start, end) = self.hunk_of(index)?;
        let mut out = self.header();
        push(&mut out, self.lines[start]);
        // The kind of line that does not exist on the side the patch applies to.
        let absent = if reverse { '-' } else { '+' };
        let mut kept_previous = true;
        for (offset, line) in self.lines[start + 1..end].iter().enumerate() {
            let at = start + 1 + offset;
            let kind = line.chars().next().unwrap_or(' ');
            let kept = if kind == '\\' {
                // "\ No newline at end of file" belongs to the line before it.
                if kept_previous {
                    push(&mut out, line);
                }
                continue;
            } else if at == index || kind == ' ' {
                push(&mut out, line);
                true
            } else if kind == absent {
                false
            } else {
                // Present on the patch's side but not chosen: unchanged context.
                push(&mut out, &format!(" {}", &line[1..]));
                true
            };
            kept_previous = kept;
        }
        Some(out)
    }
}

/// Adds one diff line, ending it with `\n` unless it already ends (`\n` or `\r\n`).
fn push(out: &mut String, line: &str) {
    out.push_str(line);
    if !line.ends_with('\n') {
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/f.txt b/f.txt
index 1111111..2222222 100644
--- a/f.txt
+++ b/f.txt
@@ -1,4 +1,4 @@
 one
-two
-three
+TWO
+THREE
 four
@@ -10,2 +10,3 @@
 ten
+ten and a half
 eleven
";

    #[test]
    fn a_hunk_patch_is_the_header_and_that_hunk() {
        let diff = FileDiff::parse(DIFF).unwrap();
        let patch = diff.hunk_patch(12).unwrap();
        assert!(patch.starts_with("diff --git a/f.txt b/f.txt\n"));
        assert!(patch.contains("@@ -10,2 +10,3 @@\n ten\n+ten and a half\n eleven\n"));
        assert!(!patch.contains("TWO"));
        assert_eq!(diff.hunk_patch(2), None, "a header line is in no hunk");
    }

    #[test]
    fn a_line_patch_keeps_one_change_like_add_p() {
        let diff = FileDiff::parse(DIFF).unwrap();
        // Stage only "+TWO": other additions drop, other removals become context.
        let stage = diff.line_patch(8, false).unwrap();
        let hunk = stage.split_once("@@ -1,4 +1,4 @@\n").unwrap().1;
        assert_eq!(hunk, " one\n two\n three\n+TWO\n four\n");
        // Unstage only "-two": other removals drop, other additions become context.
        let unstage = diff.line_patch(6, true).unwrap();
        let hunk = unstage.split_once("@@ -1,4 +1,4 @@\n").unwrap().1;
        assert_eq!(hunk, " one\n-two\n TWO\n THREE\n four\n");
        assert_eq!(diff.line_patch(5, false), None, "context is not a change");
        assert!(diff.is_change(13) && !diff.is_change(3), "--- is a header");
    }

    #[test]
    fn a_missing_newline_marker_follows_its_line() {
        let diff = FileDiff::parse(
            "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n",
        )
        .unwrap();
        // Staging only the removal drops the addition and its marker, keeping the other.
        let patch = diff.line_patch(4, false).unwrap();
        assert!(
            patch.ends_with("@@ -1 +1 @@\n-old\n\\ No newline at end of file\n"),
            "{patch}"
        );
        assert_eq!(FileDiff::parse("Binary files a/x and b/x differ\n"), None);
    }

    #[test]
    fn crlf_lines_keep_their_endings_and_renames_are_staged_whole() {
        let crlf =
            "diff --git a/w b/w\n--- a/w\n+++ b/w\n@@ -1,2 +1,2 @@\r\n one\r\n-two\r\n+TWO\r\n";
        let diff = FileDiff::parse(crlf).unwrap();
        assert!(
            diff.line_patch(6, false)
                .unwrap()
                .ends_with(" one\r\n two\r\n+TWO\r\n")
        );
        assert!(
            diff.line_patch(5, false)
                .unwrap()
                .ends_with(" one\r\n-two\r\n")
        );
        let renamed = "diff --git a/old b/new\nsimilarity index 90%\nrename from old\nrename to new\n--- a/old\n+++ b/new\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(FileDiff::parse(renamed), None);
        let conflict =
            "diff --cc f\nindex 1,2..0\n--- a/f\n+++ b/f\n@@@ -1,1 -1,1 +1,5 @@@\n++<<<<<<< HEAD\n";
        assert_eq!(FileDiff::parse(conflict), None);
        let mode = "diff --git a/s b/s\nold mode 100644\nnew mode 100755\n--- a/s\n+++ b/s\n@@ -1 +1 @@\n-a\n+b\n";
        let patch = FileDiff::parse(mode).unwrap().hunk_patch(6).unwrap();
        assert!(!patch.contains("mode"), "{patch}");
    }
}
