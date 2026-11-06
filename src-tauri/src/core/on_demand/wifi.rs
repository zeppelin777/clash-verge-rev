use crate::core::handle::Handle;
use clash_verge_logging::{Type, logging};
use objc2::{
    AnyThread as _, class, define_class, msg_send,
    rc::{Retained, autoreleasepool},
    sel,
};
use objc2_foundation::{NSArray, NSDictionary, NSObject, NSString};
use std::{cell::RefCell, ffi::c_void, ptr};

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    static NSWorkspaceDidWakeNotification: &'static NSString;
}

#[link(name = "CoreWLAN", kind = "framework")]
unsafe extern "C" {}

#[link(name = "CoreLocation", kind = "framework")]
unsafe extern "C" {}

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCDynamicStoreCopyValue(store: *const c_void, key: *const c_void) -> *mut c_void;
}

#[derive(Default)]
pub(super) enum Network {
    #[default]
    Unknown,
    PermissionRequired,
    Wifi(String),
    Other,
}

#[derive(Default)]
pub(super) struct Snapshot {
    pub(super) network: Network,
    pub(super) service: Option<String>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements for this stateless delegate.
    #[unsafe(super(NSObject))]
    #[name = "ClashVergeOnDemandNetworkDelegate"]
    struct NetworkDelegate;

    impl NetworkDelegate {
        #[unsafe(method(ssidDidChangeForWiFiInterfaceWithName:))]
        fn ssid_changed(&self, _interface: &NSString) {
            super::network_changed();
        }

        #[unsafe(method(linkDidChangeForWiFiInterfaceWithName:))]
        fn link_changed(&self, _interface: &NSString) {
            super::network_changed();
        }

        #[unsafe(method(locationManagerDidChangeAuthorization:))]
        fn authorization_changed(&self, _manager: &NSObject) {
            super::network_changed();
        }

        #[unsafe(method(systemDidWake:))]
        fn did_wake(&self, _notification: &NSObject) {
            super::network_changed();
        }
    }
);

struct Monitor {
    client: Retained<NSObject>,
    location: Retained<NSObject>,
    workspace_notifications: Retained<NSObject>,
    delegate: Retained<NetworkDelegate>,
}

// CWEventTypeSSIDDidChange and CWEventTypeLinkDidChange.
const WIFI_EVENTS: [isize; 2] = [2, 5];

thread_local! {
    static MONITOR: RefCell<Option<Monitor>> = const { RefCell::new(None) };
}

impl Monitor {
    fn new() -> Self {
        // SAFETY: The delegate and native objects are retained on the main thread. All
        // selectors and their signatures come from CoreWLAN, CoreLocation, or AppKit.
        unsafe {
            let delegate: Retained<NetworkDelegate> = msg_send![super(NetworkDelegate::alloc().set_ivars(())), init];
            let client: Retained<NSObject> = msg_send![class!(CWWiFiClient), sharedWiFiClient];
            let location: Retained<NSObject> = msg_send![class!(CLLocationManager), new];
            let workspace: Retained<NSObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
            let workspace_notifications: Retained<NSObject> = msg_send![&*workspace, notificationCenter];
            let _: () = msg_send![&*workspace_notifications, addObserver: &*delegate,
                selector: sel!(systemDidWake:), name: NSWorkspaceDidWakeNotification, object: ptr::null::<NSObject>()];
            let _: () = msg_send![&*location, setDelegate: &*delegate];
            let _: () = msg_send![&*client, setDelegate: &*delegate];
            for event in WIFI_EVENTS {
                let registered: bool = msg_send![&*client, startMonitoringEventWithType: event,
                    error: ptr::null_mut::<*mut NSObject>()];
                if !registered {
                    logging!(warn, Type::Core, "could not subscribe to CoreWLAN event {event}");
                }
            }
            Self {
                client,
                location,
                workspace_notifications,
                delegate,
            }
        }
    }

    fn sample(&self, primary: Option<&str>, request_permission: bool) -> Network {
        // SAFETY: All native objects and autoreleased values are used on the main thread.
        unsafe {
            let authorization: i32 = msg_send![&*self.location, authorizationStatus];
            if request_permission && authorization == 0 {
                let _: () = msg_send![&*self.location, requestWhenInUseAuthorization];
            }
            let Some(primary) = primary else {
                return Network::Unknown;
            };
            if is_tunnel(primary) {
                return Network::Unknown;
            }
            let interfaces: Option<Retained<NSArray<NSObject>>> = msg_send![&*self.client, interfaces];
            let Some(interfaces) = interfaces else {
                return Network::Unknown;
            };
            let interface = (0..interfaces.count())
                .map(|index| interfaces.objectAtIndex(index))
                .find(|interface| {
                    let name: Option<Retained<NSString>> = msg_send![&**interface, interfaceName];
                    name.is_some_and(|name| name.to_string() == primary)
                });
            let Some(interface) = interface else {
                return Network::Other;
            };
            let ssid: Option<Retained<NSString>> = msg_send![&*interface, ssid];
            match ssid {
                Some(ssid) => Network::Wifi(ssid.to_string()),
                None if !matches!(authorization, 3 | 4) => Network::PermissionRequired,
                None => Network::Unknown,
            }
        }
    }
}

impl Drop for Monitor {
    fn drop(&mut self) {
        // SAFETY: The monitor is only used on the main thread and still owns its delegate.
        unsafe {
            let _: () = msg_send![&*self.workspace_notifications, removeObserver: &*self.delegate];
            let _: () = msg_send![&*self.location, setDelegate: ptr::null::<NSObject>()];
            let _: bool =
                msg_send![&*self.client, stopMonitoringAllEventsAndReturnError: ptr::null_mut::<*mut NSObject>()];
            let _: () = msg_send![&*self.client, setDelegate: ptr::null::<NSObject>()];
        }
    }
}

fn is_tunnel(interface: &str) -> bool {
    ["utun", "tun", "tap", "ppp", "ipsec"]
        .iter()
        .any(|prefix| interface.starts_with(prefix))
}

fn primary_service(state_key: &str) -> Option<(String, String)> {
    let key = NSString::from_str(state_key);
    // SAFETY: These keys contain dictionaries by the SystemConfiguration schema.
    // CopyValue transfers ownership; CFDictionary and NSDictionary are toll-free bridged.
    let entry = unsafe {
        let value = SCDynamicStoreCopyValue(ptr::null(), ptr::from_ref(&*key).cast());
        Retained::<NSDictionary<NSString, NSString>>::from_raw(value.cast())
    }?;
    let interface = entry.objectForKey(&NSString::from_str("PrimaryInterface"))?;
    let service = entry.objectForKey(&NSString::from_str("PrimaryService"))?;
    Some((service.to_string(), interface.to_string()))
}

pub(super) async fn read(configured: bool, request_permission: bool) -> Snapshot {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if let Err(error) = Handle::app_handle().run_on_main_thread(move || {
        let result = autoreleasepool(|_| {
            let ipv4 = primary_service("State:/Network/Global/IPv4");
            let network = MONITOR.with_borrow_mut(|monitor| {
                if configured {
                    let ipv6 = ipv4
                        .is_none()
                        .then(|| primary_service("State:/Network/Global/IPv6"))
                        .flatten();
                    monitor.get_or_insert_with(Monitor::new).sample(
                        ipv4.as_ref().or(ipv6.as_ref()).map(|(_, interface)| interface.as_str()),
                        request_permission,
                    )
                } else {
                    *monitor = None;
                    Network::Unknown
                }
            });
            Snapshot {
                network,
                // The system proxy and the DNS scripts write to the IPv4 service.
                service: ipv4.map(|(service, _)| service),
            }
        });
        let _ = tx.send(result);
    }) {
        logging!(warn, Type::Core, "could not read the current Wi-Fi network: {error}");
        return Snapshot::default();
    }
    rx.await.unwrap_or_default()
}
