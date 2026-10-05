use std::collections::BTreeSet;

use rusqlite::{params, Connection};

use crate::{Error, Result};

const MAX_WORDS: usize = 8;
const MAX_WORD_LENGTH: usize = 64;
const MAX_VOCABULARY: usize = 50_000;
const MAX_ALTERNATIVES: usize = 32;

/// Expand a small query against indexed words, without reading image files or stored transcriptions.
pub(super) fn expression(connection: &Connection, query: &str) -> Result<Option<String>> {
    let words: BTreeSet<String> = query
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        return Ok(None);
    }
    if words.len() > MAX_WORDS
        || words
            .iter()
            .any(|word| word.chars().count() > MAX_WORD_LENGTH)
    {
        return Err(Error::InvalidInput(
            "fuzzy search supports up to 8 words of 64 characters each".into(),
        ));
    }
    let mut words: Vec<_> = words
        .into_iter()
        .map(|word| {
            let characters: Vec<_> = word.chars().collect();
            (word, characters, BTreeSet::new())
        })
        .collect();
    let lengths: Vec<_> = words
        .iter()
        .filter(|(_, characters, _)| characters.len() >= 4)
        .map(|(_, characters, _)| characters.len())
        .collect();
    if let (Some(min), Some(max)) = (lengths.iter().min(), lengths.iter().max()) {
        // A temporary vocabulary view follows the existing FTS index automatically.
        // It does not change the on-disk schema or need rebuilding after edits/scans.
        connection.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS temp.inspector_vocabulary
             USING fts5vocab(main, files_fts, 'row');",
        )?;
        let mut statement = connection.prepare(
            "SELECT term FROM temp.inspector_vocabulary
             WHERE length(term) BETWEEN ?1 AND ?2 LIMIT ?3",
        )?;
        let terms = statement.query_map(
            params![
                (*min - 1) as i64,
                (*max + 1) as i64,
                (MAX_VOCABULARY + 1) as i64
            ],
            |row| row.get::<_, String>(0),
        )?;
        for (index, term) in terms.enumerate() {
            if index == MAX_VOCABULARY {
                return Err(Error::InvalidInput(
                    "too many indexed words for this fuzzy search; narrow the query".into(),
                ));
            }
            let term = term?;
            let candidate: Vec<_> = term.chars().collect();
            for (word, characters, alternatives) in &mut words {
                if characters.len() >= 4
                    && alternatives.len() < MAX_ALTERNATIVES
                    && term != *word
                    && within_one_edit(characters, &candidate)
                {
                    alternatives.insert(term.clone());
                }
            }
        }
    }
    let expressions = words
        .into_iter()
        .map(|(word, _, alternatives)| {
            let mut terms = vec![format!("\"{word}\"*")];
            terms.extend(alternatives.into_iter().map(|term| format!("\"{term}\"")));
            format!("({})", terms.join(" OR "))
        })
        .collect::<Vec<_>>();
    Ok(Some(expressions.join(" AND ")))
}

/// One insertion, deletion, substitution, or adjacent transposition, using Unicode characters.
fn within_one_edit(left: &[char], right: &[char]) -> bool {
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
