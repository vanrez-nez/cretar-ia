use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    NotDetermined,
    Restricted,
    #[allow(dead_code)]
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct PermissionsStatus {
    pub microphone: PermissionState,
    pub accessibility: PermissionState,
}

pub fn check_permissions() -> PermissionsStatus {
    PermissionsStatus {
        microphone: check_microphone_permission(),
        accessibility: check_accessibility_permission(),
    }
}

pub fn request_microphone_permission() -> PermissionState {
    platform::request_microphone_permission()
}

pub fn request_accessibility_permission() -> PermissionState {
    platform::request_accessibility_permission()
}

fn check_microphone_permission() -> PermissionState {
    platform::check_microphone_permission()
}

fn check_accessibility_permission() -> PermissionState {
    platform::check_accessibility_permission()
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::PermissionState;

    pub fn check_microphone_permission() -> PermissionState {
        PermissionState::Unsupported
    }

    pub fn request_microphone_permission() -> PermissionState {
        PermissionState::Unsupported
    }

    pub fn check_accessibility_permission() -> PermissionState {
        PermissionState::Unsupported
    }

    pub fn request_accessibility_permission() -> PermissionState {
        PermissionState::Unsupported
    }
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
mod platform {
    use super::PermissionState;
    use block::ConcreteBlock;
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::{CFString, CFStringRef};
    use objc::class;
    use objc::msg_send;
    use objc::runtime::{Object, BOOL};
    use objc::sel;
    use objc::sel_impl;
    use std::sync::mpsc;
    use std::time::Duration;

    const AV_AUTHORIZATION_STATUS_NOT_DETERMINED: i64 = 0;
    const AV_AUTHORIZATION_STATUS_RESTRICTED: i64 = 1;
    const AV_AUTHORIZATION_STATUS_DENIED: i64 = 2;
    const AV_AUTHORIZATION_STATUS_AUTHORIZED: i64 = 3;
    const AV_MEDIA_TYPE_AUDIO: &[u8] = b"soun\0";

    #[link(name = "AVFoundation", kind = "framework")]
    extern "C" {}

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        static kAXTrustedCheckOptionPrompt: CFStringRef;
        fn AXIsProcessTrusted() -> bool;
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
    }

    pub fn check_microphone_permission() -> PermissionState {
        let media_type = audio_media_type();
        microphone_authorization_status(media_type)
    }

    pub fn request_microphone_permission() -> PermissionState {
        let media_type = audio_media_type();
        match microphone_authorization_status(media_type) {
            PermissionState::NotDetermined => request_microphone_access(media_type),
            status => status,
        }
    }

    pub fn check_accessibility_permission() -> PermissionState {
        accessibility_permission(false)
    }

    pub fn request_accessibility_permission() -> PermissionState {
        accessibility_permission(true)
    }

    fn accessibility_permission(request_prompt: bool) -> PermissionState {
        let trusted = unsafe {
            if request_prompt {
                let prompt_key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
                let prompt_value = CFBoolean::true_value();
                let options = CFDictionary::from_CFType_pairs(&[(prompt_key, prompt_value)]);
                AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef())
            } else {
                AXIsProcessTrusted()
            }
        };

        if trusted {
            PermissionState::Granted
        } else {
            PermissionState::Denied
        }
    }

    fn audio_media_type() -> *mut Object {
        unsafe { msg_send![class!(NSString), stringWithUTF8String: AV_MEDIA_TYPE_AUDIO.as_ptr()] }
    }

    fn microphone_authorization_status(media_type: *mut Object) -> PermissionState {
        let status: i64 = unsafe {
            msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: media_type]
        };
        microphone_state_from_status(status)
    }

    fn request_microphone_access(media_type: *mut Object) -> PermissionState {
        let (tx, rx) = mpsc::channel::<bool>();
        let callback = ConcreteBlock::new(move |granted: BOOL| {
            let _ = tx.send(granted);
        })
        .copy();

        unsafe {
            let _: () = msg_send![
                class!(AVCaptureDevice),
                requestAccessForMediaType: media_type
                completionHandler: &*callback
            ];
        }

        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(true) => PermissionState::Granted,
            Ok(false) => PermissionState::Denied,
            Err(_) => microphone_authorization_status(media_type),
        }
    }

    fn microphone_state_from_status(status: i64) -> PermissionState {
        match status {
            AV_AUTHORIZATION_STATUS_NOT_DETERMINED => PermissionState::NotDetermined,
            AV_AUTHORIZATION_STATUS_RESTRICTED => PermissionState::Restricted,
            AV_AUTHORIZATION_STATUS_DENIED => PermissionState::Denied,
            AV_AUTHORIZATION_STATUS_AUTHORIZED => PermissionState::Granted,
            _ => PermissionState::Unknown,
        }
    }
}
