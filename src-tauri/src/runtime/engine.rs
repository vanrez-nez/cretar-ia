use crate::audio_cues;
use crate::config::TrayConfig;
use crate::contracts::events::PipelinePhase;
use crate::contracts::status::SessionStatus;
use crate::tray;

#[derive(Clone)]
pub struct StatusRender {
    pub tooltip: String,
    pub should_pulse: bool,
    pub icon_state: &'static str,
    pub cue: Option<audio_cues::CueKind>,
}

pub fn render_status_for_host(cfg: &TrayConfig, status: &SessionStatus) -> StatusRender {
    let icon_state = tray::status_to_icon(status);
    let should_pulse =
        matches!(status.state, PipelinePhase::Starting | PipelinePhase::Recording);
    let tooltip = match status.state {
        PipelinePhase::Idle => {
            if status.source == "processing_completed" {
                cfg.tooltip.success.clone()
            } else if !status.source.is_empty() && status.error_code.is_none() {
                status.source.clone()
            } else {
                cfg.tooltip.idle.clone()
            }
        }
        PipelinePhase::Starting | PipelinePhase::Recording => cfg.tooltip.recording.clone(),
        PipelinePhase::Stopping | PipelinePhase::Processing => cfg.tooltip.sending.clone(),
        PipelinePhase::Recovering => {
            if status.source.is_empty() {
                "recovering".to_string()
            } else {
                status.source.clone()
            }
        }
        PipelinePhase::Error => {
            if status.source.is_empty() {
                "recording error".to_string()
            } else {
                status.source.clone()
            }
        }
    };

    StatusRender {
        tooltip,
        should_pulse,
        icon_state,
        cue: audio_cues::CuePlayer::status_to_cue(status),
    }
}
