//! The instance gate for request-rule evaluation (spec 0003 FR-013).
//!
//! One switch, defaulting off, read through the same settings machinery the
//! maintenance gates use. A missing or unparseable row reads as off, so losing
//! the settings table disarms request policy rather than arming it.
//!
//! There is exactly one gate here where maintenance has five. Maintenance's
//! extra four authorize *effects* of increasing blast radius — a rule that
//! deletes files needs a different acknowledgement from one that recomputes a
//! collection. A request rule has no effects: it votes, and the vote decides
//! whether a request Scryer was already going to handle is approved now or
//! looked at by a human. There is one blast radius, so there is one switch.

use scryer_domain::{AppPermission, User};

use crate::settings::keys::REQUEST_RULE_GATE_EVALUATION_KEY;
use crate::{AppResult, AppUseCase};

/// The instance-wide request-rule switches. One today; the struct exists so
/// adding a second is not an API break for WP7's GraphQL surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestRuleGates {
    pub evaluation_enabled: bool,
}

/// A partial gate update. `None` leaves the stored value alone.
#[derive(Clone, Copy, Debug, Default)]
pub struct RequestRuleGatesUpdate {
    pub evaluation_enabled: Option<bool>,
}

impl AppUseCase {
    /// Read the gate with no permission check. The submit path reads it on
    /// every evaluation, which is what makes an operator's change take effect
    /// without a restart.
    pub(crate) async fn load_request_rule_gates(&self) -> AppResult<RequestRuleGates> {
        Ok(RequestRuleGates {
            evaluation_enabled: self
                .read_setting_bool_value(REQUEST_RULE_GATE_EVALUATION_KEY, None)
                .await?
                .unwrap_or(false),
        })
    }

    /// Read the gate. Instance-wide arming is a system setting, so it is gated
    /// like one rather than like the catalog-settings authoring surface.
    pub async fn request_rule_instance_gates(&self, actor: &User) -> AppResult<RequestRuleGates> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;
        self.load_request_rule_gates().await
    }

    /// Arm or disarm the gate. Omitted fields are left exactly as stored.
    pub async fn set_request_rule_instance_gates(
        &self,
        actor: &User,
        update: RequestRuleGatesUpdate,
    ) -> AppResult<RequestRuleGates> {
        self.require_app_permission(actor, AppPermission::ManageSystemSettings)
            .await?;

        if let Some(value) = update.evaluation_enabled {
            self.upsert_system_setting_json(
                REQUEST_RULE_GATE_EVALUATION_KEY,
                &value,
                Some(actor.id.clone()),
            )
            .await?;
        }

        self.load_request_rule_gates().await
    }
}
