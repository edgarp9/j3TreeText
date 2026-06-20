use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextMatch {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceAllResult<'a> {
    pub content: Cow<'a, str>,
    pub count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplaceAllError {
    OutputTooLarge { limit: usize },
    OutputSizeOverflow,
    OutputAllocationFailed { requested: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceOneResult {
    pub content: String,
    pub replaced: TextMatch,
}

pub fn find_next_literal(content: &str, needle: &str, start: usize) -> Option<TextMatch> {
    if needle.is_empty() {
        return None;
    }

    let start = start.min(content.len());
    if !content.is_char_boundary(start) {
        return None;
    }

    let offset = content[start..].find(needle)?;
    let match_start = start + offset;
    Some(TextMatch {
        start: match_start,
        end: match_start + needle.len(),
    })
}

pub fn replace_literal_at(
    content: &str,
    needle: &str,
    replacement: &str,
    start: usize,
) -> Option<ReplaceOneResult> {
    literal_match_at(content, needle, start)?;

    let next_len = content
        .len()
        .checked_sub(needle.len())?
        .checked_add(replacement.len())?;
    let mut next_content = String::with_capacity(content.len().max(next_len));
    next_content.push_str(content);
    let replaced = replace_literal_at_in_place(&mut next_content, needle, replacement, start)?;

    Some(ReplaceOneResult {
        content: next_content,
        replaced,
    })
}

pub fn replace_literal_at_in_place(
    content: &mut String,
    needle: &str,
    replacement: &str,
    start: usize,
) -> Option<TextMatch> {
    let replaced = literal_match_at(content, needle, start)?;
    if replacement.len() > replaced.end - replaced.start {
        content
            .try_reserve(replacement.len() - (replaced.end - replaced.start))
            .ok()?;
    }
    content.replace_range(replaced.start..replaced.end, replacement);
    Some(replaced)
}

fn literal_match_at(content: &str, needle: &str, start: usize) -> Option<TextMatch> {
    if needle.is_empty() || start > content.len() || !content.is_char_boundary(start) {
        return None;
    }

    let end = start.checked_add(needle.len())?;
    if end > content.len() || !content.is_char_boundary(end) || &content[start..end] != needle {
        return None;
    }

    Some(TextMatch { start, end })
}

pub fn replace_all_literal<'a>(
    content: &'a str,
    needle: &str,
    replacement: &str,
    output_byte_limit: usize,
) -> Result<ReplaceAllResult<'a>, ReplaceAllError> {
    if needle.is_empty() {
        return Ok(ReplaceAllResult {
            content: Cow::Borrowed(content),
            count: 0,
        });
    }

    let mut parts = content.split(needle);
    let first_part = match parts.next() {
        Some(part) => part,
        None => content,
    };
    let Some(second_part) = parts.next() else {
        checked_replace_all_result_len(0, content.len(), output_byte_limit)?;
        return Ok(ReplaceAllResult {
            content: Cow::Borrowed(content),
            count: 0,
        });
    };

    if needle == replacement {
        let mut count = 1usize;
        for _ in parts {
            count = count
                .checked_add(1)
                .ok_or(ReplaceAllError::OutputSizeOverflow)?;
        }
        checked_replace_all_result_len(0, content.len(), output_byte_limit)?;
        return Ok(ReplaceAllResult {
            content: Cow::Borrowed(content),
            count,
        });
    }

    let mut result = String::new();
    let mut count = 1usize;
    push_replace_all_segment(&mut result, first_part, output_byte_limit)?;
    push_replace_all_segment(&mut result, replacement, output_byte_limit)?;
    push_replace_all_segment(&mut result, second_part, output_byte_limit)?;

    for part in parts {
        count = count
            .checked_add(1)
            .ok_or(ReplaceAllError::OutputSizeOverflow)?;
        push_replace_all_segment(&mut result, replacement, output_byte_limit)?;
        push_replace_all_segment(&mut result, part, output_byte_limit)?;
    }

    Ok(ReplaceAllResult {
        content: Cow::Owned(result),
        count,
    })
}

fn push_replace_all_segment(
    result: &mut String,
    segment: &str,
    output_byte_limit: usize,
) -> Result<usize, ReplaceAllError> {
    let next_len = checked_replace_all_result_len(result.len(), segment.len(), output_byte_limit)?;
    if result.capacity() < next_len {
        result.try_reserve(next_len - result.len()).map_err(|_| {
            ReplaceAllError::OutputAllocationFailed {
                requested: next_len,
            }
        })?;
    }
    result.push_str(segment);
    Ok(next_len)
}

fn checked_replace_all_result_len(
    current: usize,
    added: usize,
    output_byte_limit: usize,
) -> Result<usize, ReplaceAllError> {
    let next = current
        .checked_add(added)
        .ok_or(ReplaceAllError::OutputSizeOverflow)?;
    if next > output_byte_limit {
        return Err(ReplaceAllError::OutputTooLarge {
            limit: output_byte_limit,
        });
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_all_literal_borrows_content_when_no_matches() {
        let content = "large document without the target";

        let result = replace_all_literal(content, "missing", "replacement", usize::MAX)
            .expect("no-match replace all should succeed");

        assert_eq!(result.count, 0);
        assert_eq!(result.content.as_ref(), content);
        assert!(matches!(result.content, Cow::Borrowed(_)));
    }

    #[test]
    fn replace_all_literal_borrows_content_for_empty_needle() {
        let content = "unchanged content";

        let result = replace_all_literal(content, "", "replacement", 0)
            .expect("empty needle should be treated as no replacements");

        assert_eq!(result.count, 0);
        assert_eq!(result.content.as_ref(), content);
        assert!(matches!(result.content, Cow::Borrowed(_)));
    }

    #[test]
    fn replace_all_literal_replaces_matches_and_counts_them() {
        let result = replace_all_literal("one two one", "one", "three", usize::MAX)
            .expect("matching replace all should succeed");

        assert_eq!(result.count, 2);
        assert_eq!(result.content.as_ref(), "three two three");
    }

    #[test]
    fn replace_all_literal_handles_many_matches_at_exact_output_limit() {
        let content = "a".repeat(4096);
        let expected = "bc".repeat(4096);

        let result = replace_all_literal(&content, "a", "bc", expected.len())
            .expect("many-match replace all should succeed at the exact output limit");

        assert_eq!(result.count, 4096);
        assert_eq!(result.content.as_ref(), expected);
    }

    #[test]
    fn replace_all_literal_counts_identical_replacements() {
        let content = "one two one";

        let result = replace_all_literal(content, "one", "one", usize::MAX)
            .expect("identical replacement should still count matches");

        assert_eq!(result.count, 2);
        assert_eq!(result.content.as_ref(), content);
        assert!(matches!(result.content, Cow::Borrowed(_)));
    }

    #[test]
    fn replace_all_literal_identical_replacement_enforces_output_limit() {
        let result = replace_all_literal("one two one", "one", "one", 5);

        assert_eq!(result, Err(ReplaceAllError::OutputTooLarge { limit: 5 }));
    }

    #[test]
    fn replace_all_literal_enforces_output_limit_for_no_match() {
        let result = replace_all_literal("abcd", "missing", "replacement", 3);

        assert_eq!(result, Err(ReplaceAllError::OutputTooLarge { limit: 3 }));
    }

    #[test]
    fn replace_all_literal_enforces_output_limit_for_matches() {
        let result = replace_all_literal("aa", "a", "abc", 5);

        assert_eq!(result, Err(ReplaceAllError::OutputTooLarge { limit: 5 }));
    }

    #[test]
    fn replace_literal_at_in_place_reuses_existing_content() {
        let mut content = String::from("alpha beta alpha");
        let original_capacity = content.capacity();
        let replaced = replace_literal_at_in_place(&mut content, "beta", "one", 6)
            .expect("matching range should replace in place");

        assert_eq!(replaced, TextMatch { start: 6, end: 10 });
        assert_eq!(content, "alpha one alpha");
        assert!(content.capacity() <= original_capacity);
    }

    #[test]
    fn replace_literal_at_in_place_rejects_non_matching_position() {
        let mut content = String::from("alpha beta");

        assert!(replace_literal_at_in_place(&mut content, "alpha", "one", 1).is_none());
        assert_eq!(content, "alpha beta");
    }
}
