//! Shared Unicode normalization and typo-matching policy, independent of an index engine.
use std::collections::BTreeSet;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

pub(crate) fn normalize(text: &str) -> String {
    text.to_lowercase()
        .nfd()
        .filter(|c| !is_combining_mark(*c))
        .map(|c| if c == 'ς' { 'σ' } else { c })
        .collect()
}

pub(crate) fn words(text: &str) -> BTreeSet<String> {
    normalize(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// One insertion, deletion, substitution, or adjacent transposition, using Unicode characters.
pub(crate) fn within_one_edit(left: &[char], right: &[char]) -> bool {
    if left.len().abs_diff(right.len()) > 1 {
        return false;
    }
    let first = left.iter().zip(right).position(|(a, b)| a != b);
    let Some(index) = first else {
        return true;
    };
    match left.len().cmp(&right.len()) {
        std::cmp::Ordering::Less => left[index..] == right[index + 1..],
        std::cmp::Ordering::Greater => left[index + 1..] == right[index..],
        std::cmp::Ordering::Equal => {
            left[index + 1..] == right[index + 1..]
                || (index + 1 < left.len()
                    && left[index] == right[index + 1]
                    && left[index + 1] == right[index]
                    && left[index + 2..] == right[index + 2..])
        }
    }
}
