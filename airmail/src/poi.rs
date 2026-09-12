use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::language::Language;
use crate::substitutions::permute_road;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirmailPoi {
    pub source: String,
    pub admins: Vec<String>,
    pub s2cell: u64,
    pub lat: f64,
    pub lng: f64,
    pub tags: Vec<(String, String)>,
}

impl AirmailPoi {
    pub fn new(source: String, lat: f64, lng: f64, tags: Vec<(String, String)>) -> Result<Self> {
        let s2cell = s2::cellid::CellID::from(s2::latlng::LatLng::from_degrees(lat, lng)).0;

        Ok(Self {
            source,
            admins: Vec::new(),
            s2cell,
            lat,
            lng,
            tags,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ToIndexPoi {
    pub names: Vec<String>,
    pub house_number: Option<String>,
    pub road: Option<String>,
    pub unit: Option<String>,
    pub admins: Vec<String>,
    pub s2cell: u64,
    pub tags: Vec<(String, String)>,
    pub languages: Vec<Language>,
}

impl ToIndexPoi {
    pub fn new(
        names: Vec<String>,
        house_number: Option<String>,
        road: Option<String>,
        unit: Option<String>,
        lat: f64,
        lng: f64,
        tags: Vec<(String, String)>,
    ) -> Result<Self> {
        let s2cell = s2::cellid::CellID::from(s2::latlng::LatLng::from_degrees(lat, lng)).0;

        Ok(Self {
            names,
            house_number,
            road,
            unit,
            admins: Vec::new(),
            s2cell,
            tags,
            languages: Vec::new(),
        })
    }
}

pub struct SchemafiedPoi {
    pub content: Vec<String>,
    pub s2cell: u64,
    pub s2cell_parents: Vec<u64>,
    pub tags: Vec<(String, String)>,
}

impl From<ToIndexPoi> for SchemafiedPoi {
    fn from(poi: ToIndexPoi) -> Self {
        let mut content = Vec::new();
        content.extend(poi.names);
        content.extend(poi.house_number);
        if let Some(road) = poi.road {
            for lang in poi.languages {
                content.extend(permute_road(&road, &lang).expect("Failed to permute road"));
            }
        }
        content.extend(poi.unit);
        // ADOPTED: admin-hierarchy names (borough/region/locality/...)
        // dropped from content. Verified: a named POI (e.g. Times Square)
        // matches on its own name; a bare address matches on
        // house_number+road -- neither depends on this. What it costs is
        // browse-by-neighbourhood (e.g. "Theater District" alone matching
        // an unnamed POI nearby) -- accepted for a routing-to-specific-
        // places use case. -16.5% further, see report section 04.
        let _ = &poi.admins;

        // ADOPTED: routeman's wasm search path (search.worker.ts ->
        // search_index -> AirmailIndex::search(query, true, None, None, &[]))
        // hardcodes bbox=None -- construct_query's entire covering_cells
        // block is gated on `if let Some(bbox) = bbox`, so it's unreachable
        // for that call site. s2cell_parents is never term-matched by any
        // query this app actually issues; left empty rather than computed.
        let s2cell_parents = Vec::new();

        Self {
            content,
            s2cell: poi.s2cell,
            s2cell_parents,
            tags: poi.tags,
        }
    }
}
