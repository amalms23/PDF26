//! How a picker request travels to `PdfCraftActivity.pick` and how its answer comes back: plain
//! text, one field per line, so the Java side needs no JSON or array handling.
//!
//! Request: the kind (`file`, `files`, `folder` or `save`), the title, the suggested file name,
//! then the accepted extensions separated by commas (empty for any file). The answer is the chosen
//! local paths, one per line; an empty answer means cancelled.

/// The request line for one picker.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn encode(kind: &str, title: Option<&str>, file_name: Option<&str>, extensions: &[String]) -> String {
    let clean = |s: Option<&str>| s.unwrap_or_default().replace(['\n', '\r'], " ");
    let mut exts: Vec<String> = extensions.iter().map(|e| plain_extension(e)).filter(|e| !e.is_empty()).collect();
    exts.dedup();
    format!("{kind}\n{}\n{}\n{}", clean(title), clean(file_name), exts.join(","))
}

/// The chosen paths in an answer.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn decode(answer: &str) -> Vec<std::path::PathBuf> {
    answer.lines().map(str::trim).filter(|l| !l.is_empty()).map(std::path::PathBuf::from).collect()
}

/// A document name as one visible file in a folder: no path separators, no NUL, no leading dots.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
pub(crate) fn file_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '_' } else { c }).collect();
    let clean = clean.trim().trim_start_matches('.').trim();
    if clean.is_empty() { "document.pdf".into() } else { clean.into() }
}

/// `pdf` from the case-insensitive glob `[pP][dD][fF]` that `add_filter_any_case` passes to rfd,
/// lowercased; anything but letters, digits, `-` and `_` is dropped.
pub(crate) fn plain_extension(glob: &str) -> String {
    let mut out = String::new();
    let mut chars = glob.chars();
    while let Some(c) = chars.next() {
        if c == '[' {
            // Keep the first letter of the class and skip to its end.
            if let Some(first) = chars.next() {
                out.push(first);
            }
            for d in chars.by_ref() {
                if d == ']' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_become_plain_extensions() {
        assert_eq!(plain_extension("[pP][dD][fF]"), "pdf");
        assert_eq!(plain_extension("csv"), "csv");
        assert_eq!(plain_extension("*.PNG"), "png");
        assert_eq!(plain_extension("[jJ][pP]2"), "jp2");
        assert_eq!(plain_extension("[unterminated"), "u");
    }

    #[test]
    fn a_request_is_four_lines_and_newlines_in_names_cannot_add_more() {
        let r = encode("save", Some("Save\nas"), Some("a.pdf"), &["[pP][dD][fF]".into(), "pdf".into()]);
        assert_eq!(r, "save\nSave as\na.pdf\npdf");
    }

    #[test]
    fn a_shared_file_name_stays_in_its_folder() {
        assert_eq!(file_name("Notes.pdf"), "Notes.pdf");
        assert_eq!(file_name("../../etc/x.pdf"), "_.._etc_x.pdf");
        assert_eq!(file_name("a\\b\0c.pdf"), "a_b_c.pdf");
        assert_eq!(file_name("..."), "document.pdf");
        assert_eq!(file_name("  "), "document.pdf");
    }

    #[test]
    fn an_empty_answer_is_cancelled() {
        assert!(decode("").is_empty());
        assert_eq!(decode("/a/b.pdf\n\n/c.pdf\n"), vec![std::path::PathBuf::from("/a/b.pdf"), "/c.pdf".into()]);
    }
}
