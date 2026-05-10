use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct PermissionsStatus {
    pub microphone: PermissionState,
    pub accessibility: PermissionState,
}

pub async fn check_permissions() -> PermissionsStatus {
    PermissionsStatus {
        microphone: check_microphone_permission().await,
        accessibility: check_accessibility_permission().await,
    }
}

pub async fn request_microphone_permission() -> PermissionState {
    platform::request_microphone_permission().await
}

pub async fn request_accessibility_permission() -> PermissionState {
    platform::request_accessibility_permission().await
}

async fn check_microphone_permission() -> PermissionState {
    platform::check_microphone_permission().await
}

pub async fn check_accessibility_permission() -> PermissionState {
    platform::check_accessibility_permission().await
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::PermissionState;

    pub async fn check_microphone_permission() -> PermissionState {
        PermissionState::Granted
    }

    pub async fn request_microphone_permission() -> PermissionState {
        PermissionState::Granted
    }

    pub async fn check_accessibility_permission() -> PermissionState {
        PermissionState::Granted
    }

    pub async fn request_accessibility_permission() -> PermissionState {
        PermissionState::Granted
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::PermissionState;
    use tokio::time::{sleep, Duration};

    const MAX_ATTEMPTS: u8 = 3;
    const RETRY_DELAY: Duration = Duration::from_millis(200);
    const REQUEST_SETTLE_DELAY: Duration = Duration::from_millis(500);

    pub async fn check_microphone_permission() -> PermissionState {
        check_with_retry(|| async {
            tauri_plugin_macos_permissions::check_microphone_permission().await
        })
        .await
    }

    pub async fn request_microphone_permission() -> PermissionState {
        if matches!(
            check_microphone_permission().await,
            PermissionState::Granted
        ) {
            return PermissionState::Granted;
        }

        let _ = tauri_plugin_macos_permissions::request_microphone_permission().await;
        sleep(REQUEST_SETTLE_DELAY).await;
        check_microphone_permission().await
    }

    pub async fn check_accessibility_permission() -> PermissionState {
        check_with_retry(|| async {
            tauri_plugin_macos_permissions::check_accessibility_permission().await
        })
        .await
    }

    pub async fn request_accessibility_permission() -> PermissionState {
        if matches!(
            check_accessibility_permission().await,
            PermissionState::Granted
        ) {
            return PermissionState::Granted;
        }

        tauri_plugin_macos_permissions::request_accessibility_permission().await;
        sleep(REQUEST_SETTLE_DELAY).await;
        check_accessibility_permission().await
    }

    async fn check_with_retry<F, Fut>(mut check: F) -> PermissionState
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        for attempt in 1..=MAX_ATTEMPTS {
            if check().await {
                return PermissionState::Granted;
            }

            if attempt < MAX_ATTEMPTS {
                sleep(RETRY_DELAY).await;
            }
        }

        PermissionState::Denied
    }
}
