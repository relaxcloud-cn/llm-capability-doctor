mod content;
mod interface;
mod tools;

use serde_json::Value;
use thiserror::Error;

use crate::protocol::{AuthMode, Protocol, RequestSpec};

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Json(Value),
    Raw(Vec<u8>),
}

impl Body {
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::Json(value) => serde_json::to_vec(value).expect("JSON values always serialize"),
            Self::Raw(bytes) => bytes.clone(),
        }
    }

    pub fn json(&self) -> &Value {
        match self {
            Self::Json(value) => value,
            Self::Raw(_) => panic!("raw request body is not JSON"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlannedRequest {
    pub id: String,
    pub protocol: Protocol,
    pub auth_mode: AuthMode,
    pub body: Body,
    pub stream: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RequestGroup {
    Sequential(Vec<PlannedRequest>),
    Concurrent(Vec<PlannedRequest>),
}

impl RequestGroup {
    pub fn requests(&self) -> &[PlannedRequest] {
        match self {
            Self::Sequential(requests) | Self::Concurrent(requests) => requests,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestRefs {
    Executed,
    AllProtocolProbes,
    SelectedProtocolProbe,
    SharedRepeatSamples,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CheckPlan {
    pub groups: Vec<RequestGroup>,
    pub manifest_refs: ManifestRefs,
}

impl CheckPlan {
    fn executed(requests: Vec<PlannedRequest>) -> Self {
        Self {
            groups: if requests.is_empty() {
                Vec::new()
            } else {
                vec![RequestGroup::Sequential(requests)]
            },
            manifest_refs: ManifestRefs::Executed,
        }
    }

    fn references(manifest_refs: ManifestRefs) -> Self {
        Self {
            groups: Vec::new(),
            manifest_refs,
        }
    }
}

#[derive(Clone, Copy)]
pub struct PlanContext<'a> {
    pub protocol: Protocol,
    pub auth_mode: AuthMode,
    pub model: &'a str,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CheckError {
    #[error("unsupported test ID: {0}")]
    UnsupportedId(String),
}

pub fn plan(id: &str, context: &PlanContext<'_>) -> Result<CheckPlan, CheckError> {
    match id {
        "001" | "002" | "003" | "004" | "005" | "006" | "007" | "008" => {
            interface::plan(id, context)
        }
        "009" | "010" | "011" | "012" | "013" | "014" | "015" | "016" | "017" | "018" | "019"
        | "020" | "021" | "022" | "023" | "024" | "025" | "026" | "027" | "028" | "029" | "030"
        | "031" | "032" | "033" | "034" | "035" | "036" | "037" | "038" | "039" => {
            content::plan(id, context)
        }
        "040" | "041" | "042" | "043" | "044" | "045" | "046" | "047" | "048" | "049" | "050" => {
            tools::plan(id, context)
        }
        _ => Err(CheckError::UnsupportedId(id.to_owned())),
    }
}

fn from_spec(
    id: impl Into<String>,
    spec: RequestSpec,
    context: &PlanContext<'_>,
) -> PlannedRequest {
    PlannedRequest {
        id: id.into(),
        protocol: context.protocol,
        auth_mode: context.auth_mode,
        body: Body::Json(spec.body),
        stream: spec.stream,
    }
}
