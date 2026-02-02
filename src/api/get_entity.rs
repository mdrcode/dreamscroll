

/*
/// Represents either a KNode or SocialMedia entity with its associated capture
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "entity_type")]
pub enum EntityInfo {
    KNode {
        id: i32,
        name: String,
        description: String,
        k_type: String,
        capture: api::CaptureInfo,
    },
    SocialMedia {
        id: i32,
        display_name: String,
        handle: String,
        platform: String,
        capture: api::CaptureInfo,
    },
}

impl EntityInfo {
    pub fn entity_id(&self) -> i32 {
        match self {
            EntityInfo::KNode { id, .. } => *id,
            EntityInfo::SocialMedia { id, .. } => *id,
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            EntityInfo::KNode { name, .. } => name,
            EntityInfo::SocialMedia { display_name, .. } => display_name,
        }
    }

    pub fn entity_type_label(&self) -> &str {
        match self {
            EntityInfo::KNode { k_type, .. } => k_type,
            EntityInfo::SocialMedia { platform, .. } => platform,
        }
    }
}

/// Fetch a KNode entity by ID along with its associated capture
pub async fn fetch_knode(
    db: &DbHandle,
    context: &auth::Context,
    knode_id: i32,
) -> Result<Option<EntityInfo>, api::ApiError> {
    // First fetch the knode to get its capture_id
    let knode = model::knode::Entity::load()
        .filter(model::knode::Column::Id.eq(knode_id))
        .one(&db.conn)
        .await?;

    let Some(knode) = knode else {
        return Ok(None);
    };

    // Now fetch the capture with all its data (respecting user context)
    let captures = api::fetch_captures(db, context, Some(vec![knode.capture_id])).await?;
    let Some(capture) = captures.into_iter().next() else {
        return Ok(None); // User doesn't have access to this capture
    };

    Ok(Some(EntityInfo::KNode {
        id: knode.id,
        name: knode.name,
        description: knode.description,
        k_type: knode.k_type,
        capture,
    }))
}

/// Fetch a SocialMedia entity by ID along with its associated capture
pub async fn fetch_social_media(
    db: &DbHandle,
    context: &auth::Context,
    social_media_id: i32,
) -> Result<Option<EntityInfo>, api::ApiError> {
    // First fetch the social_media to get its capture_id
    let social_media = model::social_media::Entity::load()
        .filter(model::social_media::Column::Id.eq(social_media_id))
        .one(&db.conn)
        .await?;

    let Some(social_media) = social_media else {
        return Ok(None);
    };

    // Now fetch the capture with all its data (respecting user context)
    let captures = api::fetch_captures(db, context, Some(vec![social_media.capture_id])).await?;
    let Some(capture) = captures.into_iter().next() else {
        return Ok(None); // User doesn't have access to this capture
    };

    Ok(Some(EntityInfo::SocialMedia {
        id: social_media.id,
        display_name: social_media.display_name,
        handle: social_media.handle,
        platform: social_media.platform,
        capture,
    }))
}
    */
