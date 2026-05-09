use super::core_audio;
use cpal::traits::{DeviceTrait, HostTrait};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const RESTORE_TIMEOUT_MS: u64 = 2_000;
const EVENT_SETTLE_MS: u64 = 80;

#[derive(Debug, Clone)]
pub struct RouteRestoreContext {
    before: OutputRouteSnapshot,
    should_wait: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct OutputRouteSnapshot {
    device_id: u32,
    device_name: Option<String>,
    nominal_sample_rate: Option<f64>,
    stream_config_size: Option<u32>,
}

pub fn capture_restore_context(input_device: Option<&str>) -> Option<RouteRestoreContext> {
    let before = output_route_snapshot();
    let (should_wait, reason) = before.as_ref().map_or(
        (false, "missing_output_route_snapshot"),
        |snapshot| route_match_result(input_device, snapshot.device_name.as_deref()),
    );

    log::debug!(
        "media route: captured pre-recording output route should_wait={} reason={} input_device={:?} output_device={:?}",
        should_wait,
        reason,
        input_device,
        before.as_ref().and_then(|snapshot| snapshot.device_name.as_deref())
    );

    before.map(|before| RouteRestoreContext {
        before,
        should_wait,
    })
}

pub fn wait_for_restore(context: RouteRestoreContext) -> bool {
    if !context.should_wait {
        log::debug!("media route: restore gate skipped because capture did not require waiting");
        return true;
    }
    wait_for_output_route_restore(&context.before)
}

fn wait_for_output_route_restore(before: &OutputRouteSnapshot) -> bool {
    let Some(current) = output_route_snapshot() else {
        log::warn!("media route: cannot read output route before resume; proceeding");
        return true;
    };
    if route_restored(before, &current) {
        log::debug!("media route: output route already restored before={before:?} current={current:?}");
        return true;
    }

    let (tx, rx) = mpsc::channel::<()>();
    let mut listeners = core_audio::ListenerGuard::new(&tx);
    listeners.add(
        core_audio::OBJECT_SYSTEM,
        core_audio::PropertyAddress::new(
            core_audio::HARDWARE_DEFAULT_OUTPUT_DEVICE,
            core_audio::SCOPE_GLOBAL,
        ),
    );
    listeners.add(
        current.device_id,
        core_audio::PropertyAddress::new(
            core_audio::DEVICE_NOMINAL_SAMPLE_RATE,
            core_audio::SCOPE_GLOBAL,
        ),
    );
    listeners.add(
        current.device_id,
        core_audio::PropertyAddress::new(
            core_audio::DEVICE_STREAM_CONFIGURATION,
            core_audio::SCOPE_OUTPUT,
        ),
    );

    let deadline = Instant::now() + Duration::from_millis(RESTORE_TIMEOUT_MS);
    log::info!("media route: waiting for output route restore before={before:?} current={current:?}");

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let wait = remaining.min(Duration::from_millis(500));
        let _ = rx.recv_timeout(wait);
        thread::sleep(Duration::from_millis(EVENT_SETTLE_MS));

        let Some(current) = output_route_snapshot() else {
            continue;
        };
        log::debug!("media route: restore event snapshot current={current:?}");
        if route_restored(before, &current) {
            log::info!("media route: output route restored current={current:?}");
            return true;
        }
    }

    log::warn!(
        "media route: timed out waiting for output route restore before={before:?} current={:?}",
        output_route_snapshot()
    );
    false
}

fn output_route_snapshot() -> Option<OutputRouteSnapshot> {
    let device_id = core_audio::read_u32(
        core_audio::OBJECT_SYSTEM,
        core_audio::PropertyAddress::new(
            core_audio::HARDWARE_DEFAULT_OUTPUT_DEVICE,
            core_audio::SCOPE_GLOBAL,
        ),
    )?;
    Some(OutputRouteSnapshot {
        device_id,
        device_name: default_output_device_name(),
        nominal_sample_rate: core_audio::read_f64(
            device_id,
            core_audio::PropertyAddress::new(
                core_audio::DEVICE_NOMINAL_SAMPLE_RATE,
                core_audio::SCOPE_GLOBAL,
            ),
        ),
        stream_config_size: core_audio::data_size(
            device_id,
            core_audio::PropertyAddress::new(
                core_audio::DEVICE_STREAM_CONFIGURATION,
                core_audio::SCOPE_OUTPUT,
            ),
        ),
    })
}

fn route_restored(before: &OutputRouteSnapshot, current: &OutputRouteSnapshot) -> bool {
    before.device_id == current.device_id
        && before.nominal_sample_rate == current.nominal_sample_rate
        && before.stream_config_size == current.stream_config_size
}

fn default_output_device_name() -> Option<String> {
    cpal::default_host()
        .default_output_device()
        .and_then(|device| device.name().ok())
}

fn route_match_result(input_name: Option<&str>, output_name: Option<&str>) -> (bool, &'static str) {
    let Some(input) = input_name.and_then(normalize_route_name) else {
        return (false, "missing_input_device");
    };
    let Some(output) = output_name.and_then(normalize_route_name) else {
        return (false, "missing_output_device");
    };
    if input == output || input.contains(&output) || output.contains(&input) {
        (true, "input_output_match")
    } else {
        (false, "input_output_mismatch")
    }
}

fn normalize_route_name(value: &str) -> Option<String> {
    let normalized = value
        .to_ascii_lowercase()
        .replace(" microphone", "")
        .replace(" speakers", "")
        .replace(" speaker", "")
        .replace(" input", "")
        .replace(" output", "")
        .replace("’", "'")
        .trim()
        .to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}
