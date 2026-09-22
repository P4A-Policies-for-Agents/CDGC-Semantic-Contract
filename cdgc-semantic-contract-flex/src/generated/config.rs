use serde::Deserialize;
#[derive(Deserialize, Clone, Debug)]
pub struct Config {
    #[serde(alias = "annotateFieldsPresentOnly")]
    pub annotate_fields_present_only: Option<bool>,
    #[serde(
        alias = "cdgcLoginUrl",
        deserialize_with = "pdk::serde::deserialize_service"
    )]
    pub cdgc_login_url: pdk::hl::Service,
    #[serde(alias = "cdgcOrgPassword")]
    pub cdgc_org_password: String,
    #[serde(alias = "cdgcOrgUsername")]
    pub cdgc_org_username: String,
    #[serde(
        alias = "cdgcSearchUrl",
        deserialize_with = "pdk::serde::deserialize_service"
    )]
    pub cdgc_search_url: pdk::hl::Service,
    #[serde(alias = "confidentialMarkers")]
    pub confidential_markers: Option<Vec<String>>,
    #[serde(alias = "confidentialObligation")]
    pub confidential_obligation: Option<String>,
    #[serde(alias = "distributed")]
    pub distributed: Option<bool>,
    #[serde(alias = "internalMarkers")]
    pub internal_markers: Option<Vec<String>>,
    #[serde(alias = "internalObligation")]
    pub internal_obligation: Option<String>,
    #[serde(alias = "maxAnnotateBytes")]
    pub max_annotate_bytes: Option<i64>,
    #[serde(alias = "maxContractEntries")]
    pub max_contract_entries: Option<i64>,
    #[serde(alias = "maxMeaningChars")]
    pub max_meaning_chars: Option<i64>,
    #[serde(alias = "pathSchemas")]
    pub path_schemas: Option<Vec<String>>,
    #[serde(alias = "publicMarkers")]
    pub public_markers: Option<Vec<String>>,
    #[serde(alias = "publicObligation")]
    pub public_obligation: Option<String>,
    #[serde(alias = "refreshIntervalSeconds")]
    pub refresh_interval_seconds: Option<i64>,
    #[serde(alias = "restrictedMarkers")]
    pub restricted_markers: Option<Vec<String>>,
    #[serde(alias = "restrictedObligation")]
    pub restricted_obligation: Option<String>,
    #[serde(alias = "schemaId")]
    pub schema_id: Option<String>,
    #[serde(alias = "schemaIdClaim")]
    pub schema_id_claim: Option<String>,
    #[serde(alias = "schemaIdHeader")]
    pub schema_id_header: Option<String>,
    #[serde(alias = "termAttributes")]
    pub term_attributes: Option<Vec<String>>,
    #[serde(alias = "timeout")]
    pub timeout: Option<i64>,
    #[serde(alias = "toolSchemas")]
    pub tool_schemas: Option<Vec<String>>,
}
#[pdk::hl::entrypoint_flex]
fn init(abi: &dyn pdk::flex_abi::api::FlexAbi) -> Result<(), anyhow::Error> {
    let config: Config = serde_json::from_slice(abi.get_configuration())
        .map_err(|err| {
            anyhow::anyhow!(
                "Failed to parse configuration '{}'. Cause: {}",
                String::from_utf8_lossy(abi.get_configuration()), err
            )
        })?;
    abi.service_create(config.cdgc_login_url)?;
    abi.service_create(config.cdgc_search_url)?;
    abi.setup()?;
    Ok(())
}
