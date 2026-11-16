//! What changed in the soft keyboard's text buffer between two frames, as editing steps.
//!
//! GameActivity's keyboard edits a buffer the app reads (GameTextInput). Emptying that buffer
//! while the user types races with the keyboard writing the next letter, so the buffer is never
//! touched while a field has focus: each frame compares it with the previous one instead. The
//! part before the composing region is committed text; the composing region is a preedit.

/// One editing step for the focused text field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// Show `0` as the text being composed (empty: none).
    Preedit(String),
    /// Delete this many characters before the cursor.
    Delete(usize),
    /// Insert this text at the cursor.
    Commit(String),
    /// The Enter key: a line break the keyboard typed, which a one-line field (the page number,
    /// a search box) takes as "go", not as text.
    Enter,
}

/// What the app has received from the buffer so far.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Seen {
    committed: String,
    preedit: String,
}

impl Seen {
    pub(crate) fn is_empty(&self) -> bool {
        self.committed.is_empty() && self.preedit.is_empty()
    }
}

/// The byte offset of the `units`-th UTF-16 code unit in `text` (clamped to the text).
fn byte_at_utf16(text: &str, units: usize) -> usize {
    let mut seen = 0;
    for (byte, c) in text.char_indices() {
        if seen >= units {
            return byte;
        }
        seen += c.len_utf16();
    }
    text.len()
}

/// The steps that take the field from what `seen` holds to the buffer `text` with its composing
/// region (`compose`, in UTF-16 code units as Android reports it), updating `seen`.
pub(crate) fn steps(seen: &mut Seen, text: &str, compose: Option<(usize, usize)>) -> Vec<Step> {
    let (committed, preedit) = match compose {
        Some((start, end)) if end > start => {
            let (s, e) = (byte_at_utf16(text, start), byte_at_utf16(text, end));
            (text.get(..s).unwrap_or(text), text.get(s..e).unwrap_or_default())
        }
        _ => (text, ""),
    };
    let mut out = Vec::new();
    let prefix_bytes: usize = seen.committed.chars().zip(committed.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a.len_utf8()).sum();
    let removed = seen.committed.get(prefix_bytes..).map_or(0, |s| s.chars().count());
    let added = committed.get(prefix_bytes..).unwrap_or_default();
    if removed > 0 || !added.is_empty() {
        if !seen.preedit.is_empty() {
            out.push(Step::Preedit(String::new()));
            seen.preedit.clear();
        }
        if removed > 0 {
            out.push(Step::Delete(removed));
        }
        // Line breaks are the Enter key between the pieces of text around them.
        for (i, piece) in added.split('\n').enumerate() {
            if i > 0 {
                out.push(Step::Enter);
            }
            if !piece.is_empty() {
                out.push(Step::Commit(piece.to_owned()));
            }
        }
        seen.committed = committed.to_owned();
    }
    if preedit != seen.preedit {
        out.push(Step::Preedit(preedit.to_owned()));
        seen.preedit = preedit.to_owned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_typed_between_frames_all_arrive() {
        let mut seen = Seen::default();
        assert_eq!(steps(&mut seen, "He", None), vec![Step::Commit("He".into())]);
        assert_eq!(steps(&mut seen, "Hello", None), vec![Step::Commit("llo".into())]);
        assert_eq!(steps(&mut seen, "Hello", None), vec![], "nothing new, nothing sent");
    }

    #[test]
    fn composing_is_a_preedit_until_committed() {
        let mut seen = Seen::default();
        assert_eq!(steps(&mut seen, "Hi wor", Some((3, 6))), vec![Step::Commit("Hi ".into()), Step::Preedit("wor".into())]);
        assert_eq!(steps(&mut seen, "Hi world", Some((3, 8))), vec![Step::Preedit("world".into())]);
        assert_eq!(steps(&mut seen, "Hi world ", None), vec![Step::Preedit(String::new()), Step::Commit("world ".into())]);
    }

    #[test]
    fn backspace_and_autocorrect_delete_before_inserting() {
        let mut seen = Seen::default();
        steps(&mut seen, "teh ", None);
        assert_eq!(steps(&mut seen, "the ", None), vec![Step::Delete(3), Step::Commit("he ".into())]);
        assert_eq!(steps(&mut seen, "the", None), vec![Step::Delete(1)]);
    }

    #[test]
    fn a_typed_line_break_is_the_enter_key() {
        let mut seen = Seen::default();
        assert_eq!(steps(&mut seen, "12", None), vec![Step::Commit("12".into())]);
        assert_eq!(steps(&mut seen, "12\n", None), vec![Step::Enter]);
        let mut seen = Seen::default();
        assert_eq!(steps(&mut seen, "a\nb", None), vec![Step::Commit("a".into()), Step::Enter, Step::Commit("b".into())]);
    }

    #[test]
    fn regions_count_utf16_units_and_bad_regions_do_not_panic() {
        let mut seen = Seen::default();
        // "😀" is two UTF-16 units; the region after it starts at unit 2.
        assert_eq!(steps(&mut seen, "😀ab", Some((2, 4))), vec![Step::Commit("😀".into()), Step::Preedit("ab".into())]);
        let mut seen = Seen::default();
        assert_eq!(steps(&mut seen, "ab", Some((5, 9))), vec![Step::Commit("ab".into())]);
    }
}
