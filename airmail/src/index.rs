#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use geo::Rect;
use itertools::Itertools;
use log::{trace, warn};
use s2::region::RegionCoverer;
use std::collections::BTreeMap;
use tantivy::schema::Value;
#[cfg(not(target_arch = "wasm32"))]
use tantivy::directory::MmapDirectory;
use tantivy::{
    collector::{Count, TopDocs},
    query::{
        BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, PhrasePrefixQuery, PhraseQuery, Query,
        TermQuery,
    },
    schema::{
        IndexRecordOption, NumericOptions, OwnedValue, Schema, TextFieldIndexing, TextOptions,
        STORED,
    },
    Searcher, TantivyDocument, Term,
};
#[cfg(target_os = "linux")]
use tantivy_uffd::RemoteDirectory;
#[cfg(not(target_arch = "wasm32"))]
use tokio::task::spawn_blocking;
use unicode_segmentation::UnicodeSegmentation;

use crate::error::AirmailError;
use crate::{
    poi::{AirmailPoi, SchemafiedPoi},
    query::all_subsequences,
};

/// Runs a synchronous, potentially expensive closure without blocking the
/// async task driving it. Off wasm32 this offloads to tokio's blocking
/// thread pool, since tantivy's `Searcher`/`IndexReader` APIs are
/// synchronous. wasm32-unknown-unknown has no OS threads to offload to (and,
/// running inside a single-threaded Worker, nothing else to block), so there
/// it just calls the closure directly.
#[cfg(not(target_arch = "wasm32"))]
async fn run_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Ok(spawn_blocking(f).await?)
}

#[cfg(target_arch = "wasm32")]
#[allow(clippy::unused_async)]
async fn run_blocking<F, T>(f: F) -> Result<T>
where
    F: FnOnce() -> T,
{
    Ok(f())
}

// Field name keys.
pub const FIELD_CONTENT: &str = "content";
pub const FIELD_INDEXED_TAG: &str = "indexed_tag";
pub const FIELD_SOURCE: &str = "source";
pub const FIELD_S2CELL: &str = "s2cell";
pub const FIELD_S2CELL_PARENTS: &str = "s2cell_parents";
pub const FIELD_CATEGORY_JSON: &str = "category";
pub const FIELD_TAGS: &str = "tags";

#[derive(Clone)]
pub struct AirmailIndex {
    tantivy_index: Arc<tantivy::Index>,
    is_remote: bool,
}

impl AirmailIndex {
    fn schema() -> tantivy::schema::Schema {
        let mut schema_builder = Schema::builder();
        let text_options = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_fieldnorms(false)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        );
        let tag_options = TextOptions::default().set_indexing_options(
            TextFieldIndexing::default()
                .set_fieldnorms(false)
                .set_tokenizer("raw")
                .set_index_option(IndexRecordOption::Basic),
        );
        let s2cell_parent_index_options = NumericOptions::default().set_indexed();
        let s2cell_index_options = NumericOptions::default()
            .set_indexed()
            .set_stored()
            .set_fast();
        assert!(!s2cell_parent_index_options.fieldnorms());
        assert!(!s2cell_index_options.fieldnorms());

        let _ = schema_builder.add_text_field(FIELD_CONTENT, text_options.clone());
        let _ = schema_builder.add_text_field(FIELD_INDEXED_TAG, tag_options);
        let _ = schema_builder.add_text_field(FIELD_SOURCE, text_options.clone());
        let _ = schema_builder.add_u64_field(FIELD_S2CELL, s2cell_index_options);
        let _ = schema_builder.add_u64_field(FIELD_S2CELL_PARENTS, s2cell_parent_index_options);
        let _ = schema_builder.add_json_field(FIELD_TAGS, STORED);
        let _ = schema_builder.add_text_field(FIELD_CATEGORY_JSON, STORED);
        schema_builder.build()
    }

    fn field_content(&self) -> tantivy::schema::Field {
        self.tantivy_index
            .schema()
            .get_field(FIELD_CONTENT)
            .unwrap()
    }

    fn field_indexed_tag(&self) -> tantivy::schema::Field {
        self.tantivy_index
            .schema()
            .get_field(FIELD_INDEXED_TAG)
            .unwrap()
    }

    fn field_source(&self) -> tantivy::schema::Field {
        self.tantivy_index.schema().get_field(FIELD_SOURCE).unwrap()
    }

    fn field_s2cell(&self) -> tantivy::schema::Field {
        self.tantivy_index.schema().get_field(FIELD_S2CELL).unwrap()
    }

    fn field_s2cell_parents(&self) -> tantivy::schema::Field {
        self.tantivy_index
            .schema()
            .get_field(FIELD_S2CELL_PARENTS)
            .unwrap()
    }

    fn field_tags(&self) -> tantivy::schema::Field {
        self.tantivy_index.schema().get_field(FIELD_TAGS).unwrap()
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn create(index_dir: &Path) -> Result<Self> {
        if !index_dir.exists() {
            trace!("Creating index at {:?}", index_dir);
            std::fs::create_dir_all(index_dir)?;
        }

        trace!("Opening index at {:?}", index_dir);
        let schema = Self::schema();
        let tantivy_index =
            tantivy::Index::open_or_create(MmapDirectory::open(index_dir)?, schema)?;
        Ok(Self {
            tantivy_index: Arc::new(tantivy_index),
            is_remote: false,
        })
    }

    /// An in-memory index with no persisted backing storage. Doesn't need
    /// `mmap` (so it's available on wasm32), primarily useful for tests and
    /// for probing the size/behavior of the search path in isolation.
    pub fn create_in_ram() -> Result<Self> {
        let tantivy_index = tantivy::Index::create_in_ram(Self::schema());
        Ok(Self {
            tantivy_index: Arc::new(tantivy_index),
            is_remote: false,
        })
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(index_dir: &str) -> Result<Self> {
        let tantivy_index = tantivy::Index::open_in_dir(index_dir)?;
        Ok(Self {
            tantivy_index: Arc::new(tantivy_index),
            is_remote: false,
        })
    }

    /// Opens an index backed by an arbitrary tantivy `Directory` -- e.g. a
    /// custom, non-mmap `Directory` implementation such as an OPFS- or
    /// browser-storage-backed one, for platforms where `new()`/`create()`
    /// (which require the `mmap` feature) aren't available or aren't the
    /// right storage model.
    pub fn open(directory: impl Into<Box<dyn tantivy::Directory>>) -> Result<Self> {
        let tantivy_index = tantivy::Index::open(directory)?;
        Ok(Self {
            tantivy_index: Arc::new(tantivy_index),
            is_remote: false,
        })
    }

    #[cfg(target_os = "linux")]
    pub fn new_remote(base_url: &str) -> Result<Self> {
        let tantivy_index =
            tantivy::Index::open(RemoteDirectory::<{ 2 * 1024 * 1024 }>::new(base_url))?;
        Ok(Self {
            tantivy_index: Arc::new(tantivy_index),
            is_remote: true,
        })
    }

    pub fn writer(&mut self) -> Result<AirmailIndexWriter> {
        let tantivy_writer = self
            .tantivy_index
            .writer::<TantivyDocument>(2_000_000_000)?;
        let writer = AirmailIndexWriter {
            tantivy_writer,
            schema: self.tantivy_index.schema(),
        };
        Ok(writer)
    }

    pub async fn merge(&mut self) -> Result<()> {
        let ids = self.tantivy_index.searchable_segment_ids()?;
        self.tantivy_index
            .writer::<TantivyDocument>(2_000_000_000)?
            .merge(&ids)
            .await?;
        Ok(())
    }

    pub async fn num_docs(&self) -> Result<u64> {
        let index = self.tantivy_index.clone();
        let count = run_blocking(move || {
            if let Ok(tantivy_reader) = index.reader() {
                Some(tantivy_reader.searcher().num_docs())
            } else {
                None
            }
        })
        .await?;
        Ok(count.ok_or(AirmailError::UnableToCount)?)
    }

    async fn construct_query(
        &self,
        searcher: &Searcher,
        query: &str,
        tags: Option<Vec<String>>,
        bbox: Option<Rect<f64>>,
        _boost_regions: &[(f32, Rect<f64>)],
        lenient: bool,
    ) -> Box<dyn Query> {
        let mut queries: Vec<Box<dyn Query>> = Vec::new();
        let mut mandatory_queries: Vec<Box<dyn Query>> = Vec::new();

        let tokens: Vec<String> = query
            .split_word_bounds()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        for subsequence in all_subsequences(&tokens) {
            let possible_query = subsequence.join(" ");
            if possible_query
                .chars()
                .all(|c| c.is_whitespace() || c.is_ascii_punctuation())
            {
                continue;
            }

            let non_alphabetic = possible_query
                .chars()
                .filter(|c| c.is_numeric() || c.is_whitespace())
                .count();
            let total_chars = possible_query.chars().count();
            let term = Term::from_field_text(self.field_content(), &possible_query);
            let mut boost = 1.05f32.powf(possible_query.len() as f32);
            // Anecdotally, numbers in queries are usually important.
            if total_chars - non_alphabetic < 3 && non_alphabetic > 0 {
                boost *= 3.0;
            }
            if subsequence.len() > 1 {
                #[cfg(not(target_arch = "wasm32"))]
                if self.is_remote {
                    let searcher = searcher.clone();
                    let subsequence = subsequence.clone();
                    let content_field = self.field_content();
                    spawn_blocking(move || {
                        let _ = searcher.search(
                            &PhraseQuery::new(
                                subsequence
                                    .iter()
                                    .map(|s| Term::from_field_text(content_field, s))
                                    .collect(),
                            ),
                            &Count,
                        );
                    });
                }

                if self.is_remote {
                    queries.push(Box::new(BoostQuery::new(
                        Box::new(PhraseQuery::new(
                            subsequence
                                .iter()
                                .map(|s| Term::from_field_text(self.field_content(), s))
                                .collect(),
                        )),
                        boost,
                    )));
                } else {
                    queries.push(Box::new(BoostQuery::new(
                        Box::new(PhrasePrefixQuery::new(
                            subsequence
                                .iter()
                                .map(|s| Term::from_field_text(self.field_content(), s))
                                .collect(),
                        )),
                        boost,
                    )));
                }
            } else if possible_query.len() >= 8 && lenient {
                let query = if tokens.ends_with(&[possible_query]) {
                    FuzzyTermQuery::new_prefix(term, 1, true)
                } else {
                    FuzzyTermQuery::new(term, 1, true)
                };
                #[cfg(not(target_arch = "wasm32"))]
                if self.is_remote {
                    let searcher = searcher.clone();
                    let query = query.clone();
                    spawn_blocking(move || {
                        let _ = searcher.search(&query, &Count);
                    });
                }
                mandatory_queries.push(Box::new(BoostQuery::new(Box::new(query), boost)));
            } else {
                let query: Box<dyn Query> =
                    if self.is_remote || !lenient || !tokens.ends_with(&[possible_query]) {
                        Box::new(TermQuery::new(term, IndexRecordOption::Basic))
                    } else {
                        Box::new(FuzzyTermQuery::new_prefix(term, 0, false))
                    };
                #[cfg(not(target_arch = "wasm32"))]
                if self.is_remote {
                    let searcher = searcher.clone();
                    let query = query.box_clone();
                    spawn_blocking(move || {
                        let _ = searcher.search(&query, &Count);
                    });
                }
                mandatory_queries.push(Box::new(BoostQuery::new(query, boost)));
            }
        }

        if let Some(tags) = tags {
            for tag in &tags {
                let term = Term::from_field_text(self.field_indexed_tag(), tag);
                let query: Box<dyn Query> =
                    Box::new(TermQuery::new(term, IndexRecordOption::Basic));
                mandatory_queries.push(query);
            }
        }

        let optional = BooleanQuery::union(queries);
        let required = BooleanQuery::intersection(mandatory_queries);
        let final_query = BooleanQuery::new(vec![
            (Occur::Should, Box::new(optional)),
            (Occur::Must, Box::new(required)),
        ]);

        if let Some(bbox) = bbox {
            let region = s2::rect::Rect::from_degrees(
                bbox.min().y,
                bbox.min().x,
                bbox.max().y,
                bbox.max().x,
            );
            let covering_cells = {
                let coverer = RegionCoverer {
                    min_level: 0,
                    max_level: 16,
                    level_mod: 1,
                    max_cells: 64,
                };
                let mut cellunion = coverer.covering(&region);
                cellunion.normalize();
                cellunion.0.iter().map(|c| c.0).collect_vec()
            };
            let covering_disjunction_clauses = covering_cells
                .iter()
                .map(|c| {
                    let term = Term::from_field_u64(self.field_s2cell_parents(), *c);
                    let query: Box<dyn Query> =
                        Box::new(TermQuery::new(term, IndexRecordOption::Basic));
                    query
                })
                .collect_vec();
            let covering_query = BooleanQuery::union(covering_disjunction_clauses);
            return Box::new(BooleanQuery::intersection(vec![
                Box::new(covering_query),
                Box::new(final_query),
            ]));
        }

        Box::new(final_query)
    }

    /// This is public because I don't want one big mega-crate but its API should not be considered even remotely stable.
    pub async fn search(
        &self,
        query: &str,
        request_leniency: bool,
        tags: Option<Vec<String>>,
        bbox: Option<Rect<f64>>,
        boost_regions: &[(f32, Rect<f64>)],
    ) -> Result<Vec<(AirmailPoi, f32)>> {
        let tantivy_reader = self.tantivy_index.reader()?;
        let searcher = tantivy_reader.searcher();
        let query_string = query.trim().replace("'s", "s");

        // std::time::Instant::now() panics on wasm32-unknown-unknown (no
        // clock source without JS glue); it's only used for a trace log
        // below, so just skip timing there.
        #[cfg(not(target_arch = "wasm32"))]
        let start = std::time::Instant::now();

        let query = self
            .construct_query(
                &searcher,
                &query_string,
                tags,
                bbox,
                boost_regions,
                request_leniency,
            )
            .await;

        #[cfg(feature = "invasive_logging")]
        trace!("Search query: {:?}", &query);

        // Perform the search and then resolve the returned documents
        let top_docs: Result<Vec<(f32, TantivyDocument)>> = run_blocking(move || {
            let doc_addresses = searcher.search(&query, &TopDocs::with_limit(10))?;
            let mut docs = vec![];
            for (score, doc_address) in doc_addresses {
                if let Ok(doc) = searcher.doc::<TantivyDocument>(doc_address) {
                    docs.push((score, doc));
                }
            }

            Ok(docs)
        })
        .await?;

        let top_docs = top_docs.map_err(|e| {
            warn!("Search failed: {:?}", e);
            e
        })?;

        #[cfg(not(target_arch = "wasm32"))]
        trace!(
            "Search took {:?} and yielded {} results",
            start.elapsed(),
            top_docs.len()
        );

        let results = top_docs
            .into_iter()
            .flat_map(|(score, doc)| {
                let source = doc
                    .get_first(self.field_source())
                    .map(|value| value.as_str().unwrap_or_default().to_string())
                    .unwrap_or_default();
                let s2cell = doc.get_first(self.field_s2cell())?.as_u64()?;
                let cellid = s2::cellid::CellID(s2cell);
                let latlng = s2::latlng::LatLng::from(cellid);
                let tags: Vec<(String, String)> = doc
                    .get_first(self.field_tags())?
                    .as_object()?
                    .map(|(k, v)| (k.to_string(), v.as_str().unwrap_or_default().to_string()))
                    .collect();

                AirmailPoi::new(source, latlng.lat.deg(), latlng.lng.deg(), tags)
                    .ok()
                    .map(|poi| (poi, score))
            })
            .collect::<Vec<_>>();

        Ok(results)
    }
}

pub struct AirmailIndexWriter {
    tantivy_writer: tantivy::IndexWriter,
    schema: Schema,
}

impl AirmailIndexWriter {
    fn process_field(&self, doc: &mut TantivyDocument, value: &str) {
        doc.add_text(self.schema.get_field(FIELD_CONTENT).unwrap(), value);
    }

    pub fn add_poi(&mut self, poi: SchemafiedPoi, source: &str) -> Result<()> {
        let mut doc = TantivyDocument::default();
        for content in poi.content {
            self.process_field(&mut doc, &content);
        }
        doc.add_text(self.schema.get_field(FIELD_SOURCE)?, source);

        let indexed_keys = [
            "natural", "amenity", "shop", "leisure", "tourism", "historic", "cuisine",
        ];
        let indexed_key_prefixes = ["diet:"];
        for (key, value) in &poi.tags {
            if indexed_keys.contains(&key.as_str())
                || indexed_key_prefixes
                    .iter()
                    .any(|prefix| key.starts_with(prefix))
            {
                doc.add_text(
                    self.schema.get_field(FIELD_INDEXED_TAG)?,
                    format!("{}={}", key, value).as_str(),
                );
            }
        }
        doc.add_object(
            self.schema.get_field(FIELD_TAGS)?,
            poi.tags
                .iter()
                .map(|(k, v)| (k.to_string(), OwnedValue::Str(v.to_string())))
                .collect::<BTreeMap<String, OwnedValue>>(),
        );

        doc.add_u64(self.schema.get_field(FIELD_S2CELL)?, poi.s2cell);
        for parent in poi.s2cell_parents {
            doc.add_u64(self.schema.get_field(FIELD_S2CELL_PARENTS)?, parent);
        }
        self.tantivy_writer.add_document(doc)?;

        Ok(())
    }

    pub fn commit(mut self) -> Result<()> {
        self.tantivy_writer.commit()?;
        Ok(())
    }
}
