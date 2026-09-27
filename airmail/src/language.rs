/// Languages with a street-name substitution dictionary. Replaces
/// `lingua::Language` so `airmail` doesn't link lingua's multi-MB language models.
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
    /// No substitution dictionary; no substitutions are applied.
    Other,
}
