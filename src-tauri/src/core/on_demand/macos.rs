use super::{OnDemandState, OnDemandStatus, is_configured, system_proxy_enabled, wifi};
use crate::{
    config::{Config, IVerge},
    core::{
        CoreManager,
        handle::Handle,
        notify::{Refresh, announce},
        proxy_control,
        tray::Tray,
    },
    process::AsyncHandler,
    utils::resolve::dns,
};
use anyhow::Result;
use clash_verge_logging::{Type, logging, logging_error};
use parking_lot::RwLock;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::sync::Notify;

static NETWORK: RwLock<wifi::Network> = RwLock::new(wifi::Network::Unknown);
static CHANGED: Notify = Notify::const_new();
static REQUEST_PERMISSION: AtomicBool = AtomicBool::new(false);
static APPLY_FAILED: AtomicBool = AtomicBool::new(false);

pub(super) fn state(verge: &IVerge) -> OnDemandState {
    if !is_configured(verge) {
        return OnDemandState::Disabled;
    }
    let Some(rule) = &verge.on_demand else {
        return OnDemandState::Disabled;
    };
    network_state(&rule.excluded_ssids, &NETWORK.read())
}

fn network_state(excluded_ssids: &[String], network: &wifi::Network) -> OnDemandState {
    match network {
        wifi::Network::Unknown => OnDemandState::Waiting,
        wifi::Network::PermissionRequired => OnDemandState::PermissionRequired,
        wifi::Network::Wifi(ssid) if excluded_ssids.contains(ssid) => OnDemandState::Excluded,
        wifi::Network::Wifi(_) | wifi::Network::Other => OnDemandState::Allowed,
    }
}

pub(super) async fn status() -> OnDemandStatus {
    let _config_write = Config::lock_config_write().await;
    let verge = Config::verge().await.data_arc();
    let state = state(&verge);
    let paused = matches!(
        state,
        OnDemandState::Waiting | OnDemandState::PermissionRequired | OnDemandState::Excluded
    );
    let system_proxy_paused = paused
        && verge.enable_system_proxy.unwrap_or_default()
        && proxy_control::system_proxy_is_disabled().await.unwrap_or_default();
    let apply_failed = APPLY_FAILED.load(Ordering::Relaxed);
    let tun_active =
        (state != OnDemandState::Disabled || apply_failed) && CoreManager::global().applied_tun_enabled().await;
    let tun_paused = paused && verge.enable_tun_mode.unwrap_or_default() && !tun_active;
    OnDemandStatus {
        state,
        system_proxy_paused,
        tun_paused,
        tun_active,
        apply_failed,
    }
}

pub(crate) fn network_changed() {
    CHANGED.notify_one();
}

pub(crate) fn configuration_changed(request_permission: bool) {
    if request_permission {
        REQUEST_PERMISSION.store(true, Ordering::Relaxed);
    }
    network_changed();
}

pub(crate) async fn start() {
    let configured = {
        let _config_write = Config::lock_config_write().await;
        let verge = Config::verge().await.data_arc();
        let configured = is_configured(&verge);
        if configured {
            let snapshot = wifi::read(true, false).await;
            *NETWORK.write() = snapshot.network;
        }
        configured
    };
    AsyncHandler::spawn(move || async move {
        let mut was_configured = configured;
        let mut system_proxy = SystemProxy::default();
        loop {
            if was_configured {
                tokio::select! {
                    _ = CHANGED.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(15)) => {}
                }
            } else {
                CHANGED.notified().await;
            }
            // The Wi-Fi event and the primary service update can arrive in either order.
            tokio::time::sleep(Duration::from_millis(250)).await;
            if Handle::global().is_exiting() {
                break;
            }
            reconcile(&mut was_configured, &mut system_proxy).await;
        }
    });
}

#[derive(Default)]
struct SystemProxy {
    service: Option<String>,
    applied: Option<bool>,
}

impl SystemProxy {
    async fn update_service(&mut self, service: Option<String>, requested: bool, desired: bool) -> bool {
        let changed = self.service != service;
        self.service = service;
        if !requested || changed {
            self.applied = None;
        }
        if self.service.is_none() && requested && !desired {
            let _lifecycle = CoreManager::global().lifecycle_lock.lock().await;
            proxy_control::stop_guard().await;
        }
        changed
    }
}

async fn reconcile(was_configured: &mut bool, system_proxy: &mut SystemProxy) {
    let config_write = Config::lock_config_write().await;
    let verge = Config::verge().await.data_arc();
    let configured = is_configured(&verge);
    let request_permission = REQUEST_PERMISSION.swap(false, Ordering::Relaxed);
    if !configured && !*was_configured {
        return;
    }

    let snapshot = wifi::read(configured, request_permission).await;
    let previous_state = state(&verge);
    *NETWORK.write() = snapshot.network;

    let requested_proxy = verge.enable_system_proxy.unwrap_or_default();
    let desired_proxy = system_proxy_enabled(&verge);
    let service_changed = system_proxy
        .update_service(snapshot.service, requested_proxy, desired_proxy)
        .await;
    let apply_proxy = requested_proxy
        && system_proxy.service.is_some()
        && (Some(desired_proxy) != system_proxy.applied || desired_proxy != proxy_control::system_proxy_applied());
    let (tun_changed, failed) = apply_modes(
        &config_write,
        service_changed,
        apply_proxy.then_some(desired_proxy),
        system_proxy,
    )
    .await;
    let next_state = state(&verge);
    let previously_failed = APPLY_FAILED.swap(failed, Ordering::Relaxed);
    drop(config_write);

    let keep_managing = configured || failed || (requested_proxy && system_proxy.service.is_none());
    if keep_managing != *was_configured
        || previous_state != next_state
        || previously_failed != failed
        || apply_proxy
        || tun_changed
    {
        logging_error!(
            Type::Tray,
            Tray::global().update_icon(&Config::verge().await.data_arc()).await
        );
        logging_error!(Type::Tray, Tray::global().update_tooltip().await);
        announce(Refresh::Verge);
        if tun_changed {
            announce(Refresh::Clash);
        }
    }
    *was_configured = keep_managing;
    if !keep_managing {
        *system_proxy = SystemProxy::default();
    }
}

async fn apply_modes(
    config_write: &tokio::sync::MutexGuard<'_, ()>,
    service_changed: bool,
    proxy_target: Option<bool>,
    system_proxy: &mut SystemProxy,
) -> (bool, bool) {
    let mut proxy_failed = proxy_target == Some(false) && !reconcile_system_proxy(false).await;
    let manager = CoreManager::global();
    let tun_was_enabled = manager.applied_tun_enabled().await;
    let tun_result = manager.reconcile_on_demand_tun(config_write).await;
    let tun_failed = tun_result.is_err();
    logging_error!(Type::Core, tun_result);
    let tun_changed = tun_was_enabled != manager.applied_tun_enabled().await;
    if proxy_target == Some(true) {
        proxy_failed = !reconcile_system_proxy(true).await;
    }
    if let Some(target) = proxy_target
        && !proxy_failed
    {
        system_proxy.applied = Some(target);
    }
    if (service_changed || dns::sync_failed()) && !tun_changed && (proxy_target != Some(true) || proxy_failed) {
        let _lifecycle = manager.lifecycle_lock.lock().await;
        dns::sync_public_dns().await;
    }
    (tun_changed, proxy_failed || tun_failed || dns::sync_failed())
}

async fn reconcile_system_proxy(enabled: bool) -> bool {
    async fn apply(enabled: bool) -> Result<()> {
        let manager = CoreManager::global();
        let _lifecycle = manager.lifecycle_lock.lock().await;
        if !proxy_control::has_network_service().await {
            if !enabled {
                proxy_control::stop_guard().await;
            }
            anyhow::bail!("waiting for a network service");
        }
        if enabled {
            manager.apply_proxy_after_start().await?;
            anyhow::ensure!(proxy_control::system_proxy_applied(), "system proxy was not restored");
        } else {
            proxy_control::apply().await?;
            proxy_control::refresh_guard().await?;
        }
        let disabled = proxy_control::system_proxy_is_disabled().await?;
        anyhow::ensure!(enabled != disabled, "system proxy did not reach its requested state");
        Ok(())
    }
    match apply(enabled).await {
        Ok(()) => true,
        Err(error) => {
            logging!(
                warn,
                Type::Core,
                "failed to apply on-demand system proxy state: {error:#}"
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluded_names_match_exactly_and_unidentified_wifi_does_not_resume() {
        let excluded = [String::from(" Home, 5G; ")];
        for (network, expected) in [
            (wifi::Network::Wifi(" Home, 5G; ".into()), OnDemandState::Excluded),
            (wifi::Network::Wifi(" home, 5G; ".into()), OnDemandState::Allowed),
            (wifi::Network::Wifi("Home, 5G;".into()), OnDemandState::Allowed),
            (wifi::Network::PermissionRequired, OnDemandState::PermissionRequired),
            (wifi::Network::Unknown, OnDemandState::Waiting),
            (wifi::Network::Other, OnDemandState::Allowed),
        ] {
            assert_eq!(network_state(&excluded, &network), expected);
        }
    }

    #[test]
    fn leaving_an_excluded_network_restores_only_the_latest_preferences() {
        use crate::{config::OnDemandConfig, core::on_demand::tun_enabled};

        let original = std::mem::take(&mut *NETWORK.write());
        let _restore = scopeguard::guard(original, |original| *NETWORK.write() = original);
        let mut verge = IVerge {
            on_demand: Some(OnDemandConfig {
                enabled: true,
                excluded_ssids: vec!["Home".into()],
            }),
            ..IVerge::default()
        };

        for (proxy, tun) in [(true, false), (false, true), (true, true), (false, false)] {
            verge.patch_config(&IVerge {
                enable_system_proxy: Some(proxy),
                enable_tun_mode: Some(tun),
                ..IVerge::default()
            });
            for network in [wifi::Network::Unknown, wifi::Network::Wifi("Home".into())] {
                *NETWORK.write() = network;
                assert_eq!((system_proxy_enabled(&verge), tun_enabled(&verge)), (false, false));
            }
            *NETWORK.write() = wifi::Network::Wifi("Other".into());
            assert_eq!((system_proxy_enabled(&verge), tun_enabled(&verge)), (proxy, tun));
        }

        *NETWORK.write() = wifi::Network::Wifi("Home".into());
        verge.patch_config(&IVerge {
            enable_system_proxy: Some(true),
            enable_tun_mode: Some(true),
            ..IVerge::default()
        });
        assert_eq!((system_proxy_enabled(&verge), tun_enabled(&verge)), (false, false));
        verge.patch_config(&IVerge {
            enable_tun_mode: Some(false),
            ..IVerge::default()
        });
        *NETWORK.write() = wifi::Network::Other;
        assert_eq!((system_proxy_enabled(&verge), tun_enabled(&verge)), (true, false));
    }
}
