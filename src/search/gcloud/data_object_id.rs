use anyhow::Context;

use crate::search;

/// Ideally, we would use set_output_fields on the DataObjectSearchRequest
/// to configure the projection of search fields, but (BUG?) every attempt to
/// use set_output_fields results in a 400 Bad Request with message "invalid
/// argument". So as a hack/workaround, we encode the fields we need (user_id,
/// capture_id, illumination_id) in the document ID and parse them back at
/// search time.

pub(crate) fn make<E>(embed: &search::CaptureEmbedding<E>) -> String {
    make_from_fields(embed.user_id, embed.capture_id, embed.illumination_id)
}

pub(crate) fn make_from_fields(user_id: i32, capture_id: i32, illumination_id: i32) -> String {
    format!("u{}-c{}-i{}", user_id, capture_id, illumination_id)
}

pub(crate) fn parse_fields(doc_id: &str) -> anyhow::Result<(i32, i32, i32)> {
    // Expected format from vector upsert path: u<user_id>-c<capture_id>-i<illumination_id>
    let mut parts = doc_id.split('-');
    let user = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("data_object_id missing user"))?;
    let capture = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("data_object_id missing capture"))?;
    let illumination = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("data_object_id missing illumination"))?;

    if parts.next().is_some() {
        anyhow::bail!("unexpected extra doc_id segments");
    }

    let user_id = user
        .strip_prefix('u')
        .ok_or_else(|| anyhow::anyhow!("data_object_id user missing 'u' prefix"))?
        .parse::<i32>()
        .context("user id is not an integer")?;
    let capture_id = capture
        .strip_prefix('c')
        .ok_or_else(|| anyhow::anyhow!("data_object_id capture missing 'c' prefix"))?
        .parse::<i32>()
        .context("capture id is not an integer")?;
    let illumination_id = illumination
        .strip_prefix('i')
        .ok_or_else(|| anyhow::anyhow!("data_object_id illumination missing 'i' prefix"))?
        .parse::<i32>()
        .context("illumination id is not an integer")?;

    Ok((user_id, capture_id, illumination_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_doc_id_extracts_user_capture_and_illumination() {
        assert_eq!(parse_fields("u1-c123-i456").ok(), Some((1, 123, 456)));
        assert!(parse_fields("u1-cabc-i2").is_err());
        assert!(parse_fields("ufoo-c1-i2").is_err());
        assert!(parse_fields("bad").is_err());
    }
}
