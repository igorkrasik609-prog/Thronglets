use crate::identity::IdentityBinding;
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct IdentityRoleBlueprint {
    pub definition: &'static str,
    pub current_v1_binding: &'static str,
    pub current_id: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct IdentityBlueprint {
    pub version: &'static str,
    pub authorization_truth_source: &'static str,
    pub public_finality_layer: &'static str,
    pub principal: IdentityRoleBlueprint,
    pub account: IdentityRoleBlueprint,
    pub delegate: IdentityRoleBlueprint,
    pub session: IdentityRoleBlueprint,
}

#[derive(Clone, Serialize)]
pub struct AuthorizationSummary {
    pub final_truth_source: &'static str,
    pub local_binding_status: &'static str,
    pub local_binding_source: String,
    pub authoritative_status: &'static str,
    pub execution_boundary: &'static str,
}

#[derive(Clone, Serialize)]
pub struct IdentitySummary {
    pub status: &'static str,
    pub owner_account: Option<String>,
    pub device_identity: String,
    pub binding_source: String,
    pub joined_from_device: Option<String>,
    pub identity_model: IdentityBlueprint,
    pub authorization: AuthorizationSummary,
}

#[derive(Clone, Serialize)]
pub struct AuthorizationCheckData {
    pub summary: AuthorizationSummary,
    pub owner_account: Option<String>,
    pub device_identity: String,
    pub binding_source: String,
    pub joined_from_device: Option<String>,
    pub identity_model: IdentityBlueprint,
}

pub fn identity_blueprint(
    owner_account: Option<String>,
    device_identity: String,
) -> IdentityBlueprint {
    IdentityBlueprint {
        version: "principal-account-delegate-session.v1",
        authorization_truth_source: "oasyce_chain",
        public_finality_layer: "oasyce_chain",
        principal: IdentityRoleBlueprint {
            definition: "continuous subject",
            current_v1_binding: "not-modeled-in-v1",
            current_id: None,
        },
        account: IdentityRoleBlueprint {
            definition: "asset / settlement container",
            current_v1_binding: "owner_account",
            current_id: owner_account,
        },
        delegate: IdentityRoleBlueprint {
            definition: "authorized executor",
            current_v1_binding: "device_identity",
            current_id: Some(device_identity),
        },
        session: IdentityRoleBlueprint {
            definition: "one concrete run; never an economic subject",
            current_v1_binding: "session_id_audit_label",
            current_id: None,
        },
    }
}

pub fn authorization_summary(binding: &IdentityBinding) -> AuthorizationSummary {
    AuthorizationSummary {
        final_truth_source: "oasyce_chain",
        local_binding_status: if binding.owner_account.is_some() {
            "owner-bound"
        } else {
            "unbound"
        },
        local_binding_source: binding.binding_source_or_local().to_string(),
        authoritative_status: "not-checked",
        execution_boundary: "device_identity",
    }
}

pub fn identity_summary(status: &'static str, binding: &IdentityBinding) -> IdentitySummary {
    IdentitySummary {
        status,
        owner_account: binding.owner_account.clone(),
        device_identity: binding.device_identity.clone(),
        binding_source: binding.binding_source_or_local().to_string(),
        joined_from_device: binding.joined_from_device.clone(),
        identity_model: identity_blueprint(
            binding.owner_account.clone(),
            binding.device_identity.clone(),
        ),
        authorization: authorization_summary(binding),
    }
}

pub fn authorization_check_data(binding: &IdentityBinding) -> AuthorizationCheckData {
    AuthorizationCheckData {
        summary: authorization_summary(binding),
        owner_account: binding.owner_account.clone(),
        device_identity: binding.device_identity.clone(),
        binding_source: binding.binding_source_or_local().to_string(),
        joined_from_device: binding.joined_from_device.clone(),
        identity_model: identity_blueprint(
            binding.owner_account.clone(),
            binding.device_identity.clone(),
        ),
    }
}
