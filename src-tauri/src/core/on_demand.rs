use crate::config::IVerge;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod wifi;

#[cfg(target_os = "macos")]
pub(crate) use macos::{configuration_changed, network_changed, start};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum OnDemandState {
    #[default]
    Disabled,
    Waiting,
    PermissionRequired,
    Excluded,
    Allowed,
}

#[derive(Default, serde::Serialize, specta::Type)]
pub struct OnDemandStatus {
    pub state: OnDemandState,
    pub system_proxy_paused: bool,
    pub tun_paused: bool,
    pub tun_active: bool,
    pub apply_failed: bool,
}

pub(crate) fn is_configured(verge: &IVerge) -> bool {
    cfg!(target_os = "macos")
        && verge
            .on_demand
            .as_ref()
            .is_some_and(|rule| rule.enabled && !rule.excluded_ssids.is_empty())
}

fn allows_proxy(verge: &IVerge) -> bool {
    if !is_configured(verge) {
        return true;
    }
    #[cfg(target_os = "macos")]
    return macos::state(verge) == OnDemandState::Allowed;
    #[cfg(not(target_os = "macos"))]
    true
}

pub(crate) fn system_proxy_enabled(verge: &IVerge) -> bool {
    verge.enable_system_proxy.unwrap_or_default() && allows_proxy(verge)
}

pub(crate) fn tun_enabled(verge: &IVerge) -> bool {
    verge.enable_tun_mode.unwrap_or_default() && allows_proxy(verge)
}

pub async fn status() -> OnDemandStatus {
    #[cfg(target_os = "macos")]
    return macos::status().await;
    #[cfg(not(target_os = "macos"))]
    OnDemandStatus::default()
}
