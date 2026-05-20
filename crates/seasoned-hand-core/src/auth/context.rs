//! Auth context and role/action contracts.
//!
//! refs: /specs/phase-5/architecture.md §4.1, §4.4

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    User,
    Viewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    TaskRead,
    TaskWrite,
    TaskHandoff,
    SopShare,
    PlaybookShare,
    MembershipManage,
    AuditRead,
    EventRawRead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthContext {
    pub tenant_id: String,
    pub organization_id: String,
    pub actor_user_id: String,
    pub org_role: Role,
    pub project_override_role: Option<Role>,
}

impl AuthContext {
    pub fn effective_role(&self) -> Role {
        effective_role(self.org_role, self.project_override_role)
    }
}

pub fn effective_role(org_role: Role, project_override_role: Option<Role>) -> Role {
    project_override_role.unwrap_or(org_role)
}
