use super::{
    store::{storage_error, State, Store},
    Database,
};
use crate::domain::search::{normalize, within_one_edit, words};
use crate::{Error, FileRecord, Result, SearchHit, SearchPage};
use std::collections::{BTreeMap, BTreeSet};
use wedb_embed::{InvertedIndex, SearchIndexSchema};

pub(super) struct SearchIndex {
    inverted: InvertedIndex,
    schema: SearchIndexSchema,
    // Ordered dictionary permits prefix ranges; lengths bound typo expansion.
    dictionary: BTreeMap<usize, BTreeMap<String, usize>>,
    body_files: BTreeMap<String, BTreeSet<i64>>,
    file_bodies: BTreeMap<i64, String>,
}
impl SearchIndex {
    pub fn load(store: &Store, state: &State) -> Result<Self> {
        let mut index = Self {
            inverted: InvertedIndex::new(),
            schema: SearchIndexSchema::new("files", vec![], vec!["path".into(), "body".into()]),
            dictionary: BTreeMap::new(),
            body_files: BTreeMap::new(),
            file_bodies: BTreeMap::new(),
        };
        for file in state.files.values() {
            index.upsert(store, file)?;
        }
        Ok(index)
    }
    fn terms(&self, id: &str) -> BTreeSet<String> {
        self.inverted
            .docs
            .get(id)
            .map(|(fields, _, _)| fields.values().flat_map(|text| words(text)).collect())
            .unwrap_or_default()
    }
    fn remove_doc(&mut self, id: &str) {
        for term in self.terms(id) {
            let length = term.chars().count();
            if let Some(terms) = self.dictionary.get_mut(&length) {
                if let Some(count) = terms.get_mut(&term) {
                    *count -= 1;
                    if *count == 0 {
                        terms.remove(&term);
                    }
                }
                if terms.is_empty() {
                    self.dictionary.remove(&length);
                }
            }
        }
        self.inverted.delete_doc(&self.schema, id);
    }
    pub fn remove(&mut self, id: i64) {
        self.remove_doc(&id.to_string());
        if let Some(body) = self.file_bodies.remove(&id) {
            if let Some(files) = self.body_files.get_mut(&body) {
                files.remove(&id);
                if files.is_empty() {
                    self.body_files.remove(&body);
                    self.remove_doc(&body);
                }
            }
        }
    }
    fn add_doc(&mut self, id: &str, fields: &serde_json::Value) -> Result<()> {
        self.inverted
            .index_doc(&self.schema, id, &serde_json::to_vec(fields)?, None, None)
            .map_err(storage_error)?;
        for term in self.terms(id) {
            *self
                .dictionary
                .entry(term.chars().count())
                .or_default()
                .entry(term)
                .or_default() += 1;
        }
        Ok(())
    }
    pub fn upsert(&mut self, store: &Store, file: &FileRecord) -> Result<()> {
        self.remove(file.id);
        self.add_doc(
            &file.id.to_string(),
            &serde_json::json!({"path":normalize(&file.relative_path).replace('_'," ")}),
        )?;
        if let Some(hash) = &file.text_hash {
            let body_id = format!("text:{hash}");
            if !self.body_files.contains_key(&body_id) {
                let body = store
                    .get::<String>(&format!("texts/{hash}"))?
                    .ok_or_else(|| Error::Storage("missing indexed text body".into()))?;
                self.add_doc(
                    &body_id,
                    &serde_json::json!({"body":normalize(&body).replace('_'," ")}),
                )?;
            }
            self.body_files
                .entry(body_id.clone())
                .or_default()
                .insert(file.id);
            self.file_bodies.insert(file.id, body_id);
        }
        Ok(())
    }
    fn match_terms(&self, query: &str) -> (BTreeSet<i64>, bool, Option<String>, BTreeSet<String>) {
        let query = words(query);
        if query.is_empty() {
            return (BTreeSet::new(), false, None, BTreeSet::new());
        }
        let mut reason = None;
        if query.len() > 8 || query.iter().any(|word| word.chars().count() > 64) {
            reason = Some("fuzzy search supports up to 8 words of 64 characters each");
        }
        let lengths = query
            .iter()
            .map(|word| word.chars().count())
            .filter(|len| *len >= 4)
            .collect::<Vec<_>>();
        if reason.is_none() && !lengths.is_empty() {
            let min = *lengths.iter().min().unwrap() - 1;
            let max = *lengths.iter().max().unwrap() + 1;
            if self
                .dictionary
                .range(min..=max)
                .map(|(_, terms)| terms.len())
                .sum::<usize>()
                > 50_000
            {
                reason = Some("too many indexed words for this fuzzy search; narrow the query");
            }
        }
        let mut intersection: Option<BTreeSet<i64>> = None;
        let mut highlighted = BTreeSet::new();
        for word in query {
            let mut expanded = BTreeSet::new();
            expanded.insert(word.clone());
            if reason.is_none() {
                for terms in self.dictionary.values() {
                    for (term, _) in terms.range(word.clone()..) {
                        if !term.starts_with(&word) {
                            break;
                        }
                        expanded.insert(term.clone());
                    }
                }
                let characters = word.chars().collect::<Vec<_>>();
                if characters.len() >= 4 {
                    let mut alternatives = BTreeSet::new();
                    for (_, terms) in self
                        .dictionary
                        .range(characters.len() - 1..=characters.len() + 1)
                    {
                        for term in terms.keys() {
                            if term != &word
                                && within_one_edit(&characters, &term.chars().collect::<Vec<_>>())
                            {
                                alternatives.insert(term.clone());
                            }
                        }
                    }
                    expanded.extend(alternatives.into_iter().take(32));
                }
            }
            let mut matching = BTreeSet::new();
            for field in self.inverted.text_index.values() {
                for term in &expanded {
                    if let Some(postings) = field.get(term) {
                        for id in postings.keys() {
                            if let Ok(id) = id.parse::<i64>() {
                                matching.insert(id);
                            } else if let Some(files) = self.body_files.get(id.as_str()) {
                                matching.extend(files);
                            }
                        }
                    }
                }
            }
            highlighted.extend(expanded);
            intersection = Some(intersection.map_or(matching.clone(), |old| {
                old.intersection(&matching).copied().collect()
            }));
        }
        (
            intersection.unwrap_or_default(),
            reason.is_none(),
            reason.map(|reason| format!("Showing exact matches: {reason}")),
            highlighted,
        )
    }
    fn literal_rank(&self, id: i64, query: &str) -> Option<u8> {
        let fields = &self.inverted.docs.get(id.to_string().as_str())?.0;
        if fields.get("path").is_some_and(|path| path.contains(query)) {
            Some(0)
        } else if self
            .file_bodies
            .get(&id)
            .and_then(|body| self.inverted.docs.get(body.as_str()))
            .and_then(|doc| doc.0.get("body"))
            .is_some_and(|body| body.contains(query))
        {
            Some(1)
        } else {
            None
        }
    }
}
pub(crate) struct SearchRepository<'a> {
    database: &'a Database,
}
impl<'a> SearchRepository<'a> {
    pub(super) fn new(database: &'a Database) -> Self {
        Self { database }
    }
    pub fn query(&self, text: &str, folder: Option<i64>, limit: usize) -> Result<Vec<SearchHit>> {
        if text.trim().is_empty() {
            return Ok(vec![]);
        }
        Ok(self.page(text, folder, 0, limit, true)?.hits)
    }
    pub fn page(
        &self,
        text: &str,
        folder: Option<i64>,
        offset: usize,
        limit: usize,
        fuzzy: bool,
    ) -> Result<SearchPage> {
        if text.len() > 4096 {
            return Err(Error::InvalidInput(
                "search query exceeds 4096 bytes".into(),
            ));
        }
        let text = text.trim();
        let literal = normalize(text).replace('_', " ");
        let index = self.database.index.borrow();
        let (matches, fuzzy_applied, notice, highlighted) = if fuzzy {
            index.match_terms(text)
        } else {
            (BTreeSet::new(), false, None, BTreeSet::new())
        };
        let state = self.database.state.borrow();
        let mut candidates = state
            .files
            .values()
            .filter(|file| folder.is_none_or(|folder| file.folder_id == folder))
            .filter_map(|file| {
                let rank = index.literal_rank(file.id, &literal);
                rank.or_else(|| matches.contains(&file.id).then_some(2))
                    .map(|rank| (rank, file))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|(ra, a), (rb, b)| {
            ra.cmp(rb)
                .then(a.relative_path.cmp(&b.relative_path))
                .then(a.folder_id.cmp(&b.folder_id))
                .then(a.id.cmp(&b.id))
        });
        let total = candidates.len();
        let hits = candidates
            .into_iter()
            .skip(offset)
            .take(limit.clamp(1, 100))
            .map(|(_, file)| {
                let details = self.database.files().get(file.id)?;
                let source = details
                    .text
                    .as_deref()
                    .filter(|body| {
                        normalize(body).contains(&literal)
                            || words(body).iter().any(|word| highlighted.contains(word))
                    })
                    .unwrap_or(&file.relative_path);
                Ok(SearchHit {
                    file: file.clone(),
                    folder_path: state.folder(file.folder_id)?.path.clone(),
                    snippet: snippet(source, &literal, &highlighted),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(SearchPage {
            hits,
            total,
            fuzzy_applied,
            notice,
        })
    }
    pub fn rebuild(&self) -> Result<()> {
        *self.database.index.borrow_mut() =
            SearchIndex::load(&self.database.store, &self.database.state.borrow())?;
        Ok(())
    }
}
fn snippet(source: &str, literal: &str, terms: &BTreeSet<String>) -> String {
    let chars = source.chars().collect::<Vec<_>>();
    let mut spans = Vec::new();
    let mut start = None;
    for (index, ch) in chars
        .iter()
        .copied()
        .chain(std::iter::once(' '))
        .enumerate()
    {
        if ch.is_alphanumeric() {
            start.get_or_insert(index);
        } else if let Some(begin) = start.take() {
            let word = normalize(&chars[begin..index].iter().collect::<String>());
            if terms.contains(&word) {
                spans.push((begin, index));
            }
        }
    }
    let first = spans.first().map_or_else(
        || {
            if literal.is_empty() {
                0
            } else {
                source
                    .to_lowercase()
                    .find(literal)
                    .map(|byte| source.to_lowercase()[..byte].chars().count())
                    .unwrap_or(0)
            }
        },
        |span| span.0,
    );
    let begin = first.saturating_sub(80);
    let end = (begin + 240).min(chars.len());
    let mut output = String::new();
    if begin > 0 {
        output.push_str("… ");
    }
    for (index, ch) in chars.iter().enumerate().take(end).skip(begin) {
        if spans.iter().any(|(start, _)| *start == index) {
            output.push('[');
        }
        output.push(*ch);
        if spans.iter().any(|(_, end)| *end == index + 1) {
            output.push(']');
        }
    }
    if end < chars.len() {
        output.push_str(" …");
    }
    output
}
