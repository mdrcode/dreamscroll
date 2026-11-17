//! What one resolve run asked, wrote, refused, or would write.

use crate::fetch;

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Ask Wikidata again even when it already annotated the entity. `--entity` sets this;
    /// without it, the already-annotated filter makes an entity-scoped run empty.
    pub include_already_annotated: bool,
    /// Report what a positive answer would write, without writing or projecting it.
    pub dry_run: bool,
}

/// One identity a dry run would write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunIdentity {
    pub entity_id: i32,
    pub provider: String,
    pub scheme: String,
    pub external_id: String,
    pub method: Option<String>,
}

/// One resolution run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ResolutionReport {
    pub offline: bool,
    /// No identities were written or projected; the counts are what the run would do. Provider
    /// answers may still populate `fetch_cache`.
    pub dry_run: bool,

    pub considered: usize,
    pub fetch_traffic: fetch::Stats,

    pub entities_resolved: usize,
    pub identities_written: usize,
    pub identities_vetoed: usize,
    pub identities_replaced: usize,

    pub by_provider: Vec<ProviderReport>,
    /// What a dry run would write; empty for a real run.
    pub preview: Vec<DryRunIdentity>,
    /// The run-wide budget refusal, or a provider's rate limit, for the report.
    pub refusal: Option<fetch::Refusal>,
}

/// What Wikidata did in one run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProviderReport {
    pub provider: String,
    pub batch: usize,
    pub fetch_traffic: fetch::Stats,

    pub identities_written: usize,
    pub identities_vetoed: usize,
    /// Old annotations this provider owned that the run replaced.
    pub identities_replaced: usize,
}
