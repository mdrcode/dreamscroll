use crate::{api, auth, illumination::Illuminator};

use super::*;

pub fn make_processor(
    service_api: crate::api::ServiceApiClient,
    illuminator: Box<dyn Illuminator>,
) -> CaptureIlluminationProcessor {
    CaptureIlluminationProcessor::new(service_api, illuminator)
}

