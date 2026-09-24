use serde::{Deserialize, Serialize};

/// A measurement made by the normal import admission check, never a second probe.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceMeasurement {
    pub destination_key: String,
    pub destination: String,
    pub available_bytes: u64,
    pub required_bytes: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpaceIncidentEvent {
    pub incident_id: String,
    pub measurement: SpaceMeasurement,
    pub affected_import_count: usize,
    pub recovered: bool,
}

#[derive(Clone, Debug)]
pub enum SpaceIncidentUpdate {
    Observed {
        member_key: String,
        job_key: String,
        download_id: Option<String>,
        measurement: SpaceMeasurement,
        blocked: bool,
    },
    Retired {
        job_key: String,
        download_id: Option<String>,
    },
}
