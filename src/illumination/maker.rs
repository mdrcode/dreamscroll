use crate::{config, storage};

use super::*;

pub fn make_illuminator(
    cfg: &config::Config,
    storage: Box<dyn storage::StorageProvider>,
) -> anyhow::Result<Box<dyn Illuminator>> {
    let illuminator: Box<dyn Illuminator> = match cfg.illuminator.as_str() {
        "gemini" => Box::new(gemini::GeminiIlluminator::new(cfg, storage)?),
        "grok" => Box::new(grok::GrokIlluminator::default()),
        "loremipsum" => Box::new(loremipsum::LoremIpsumIlluminator),
        other => unimplemented!(
            "Unknown illuminator model '{}'. Supported: gemini, grok, loremipsum.",
            other
        ),
    };
    Ok(illuminator)
}
