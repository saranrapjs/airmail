/// The languages `airmail` itself distinguishes for street-name substitution
/// (see `substitutions::permute_road`). This is deliberately a small, local
/// enum rather than `lingua::Language`: `airmail` never runs lingua's
/// statistical language *detection* (only `airmail_indexer` does, at import
/// time), it only ever matches on a handful of variants to pick a
/// substitution dictionary, and `lingua`'s per-language Cargo features each
/// pull several MB of bundled n-gram model data into any binary that enables
/// them -- data this crate never uses. `airmail_indexer` still depends on
/// `lingua` directly for detection, and maps its richer `Language`/
/// `IsoCode639_3` into this enum at the crate boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Arabic,
    Spanish,
    French,
    German,
    Italian,
    Portuguese,
    Russian,
    Chinese,
    Catalan,
    /// Any detected language `airmail` doesn't have a substitution
    /// dictionary for. Falls back to no substitutions in `permute_road`.
    Other,
}
