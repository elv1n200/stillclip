//! Capture intent survives exports, device probing and start/stop transitions.
#[derive(Default)]
pub struct CaptureControl {
    pub transitioning: bool,
    desired: Option<bool>,
}

impl CaptureControl {
    pub fn request(&mut self, running: bool) {
        self.desired = Some(running);
    }

    pub fn wants_start(&self) -> bool {
        self.desired == Some(true)
    }

    pub fn next(&mut self, running: bool) -> Option<bool> {
        if self.transitioning {
            return None;
        }
        self.desired.take().filter(|wanted| *wanted != running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_during_start_is_delivered_after_start_completes() {
        let mut control = CaptureControl::default();
        control.request(true);
        assert_eq!(control.next(false), Some(true));
        control.transitioning = true;
        control.request(false);
        assert_eq!(control.next(false), None);
        control.transitioning = false;
        assert_eq!(control.next(true), Some(false));
        assert_eq!(control.next(false), None);
    }

    #[test]
    fn game_ending_cancels_a_start_waiting_for_device_probing() {
        let mut control = CaptureControl::default();
        control.request(true);
        assert!(control.wants_start());
        control.request(false);
        assert!(!control.wants_start());
        assert_eq!(control.next(false), None);
    }

    #[test]
    fn latest_intent_wins_without_automatically_retrying_failed_starts() {
        let mut control = CaptureControl::default();
        control.request(false);
        control.request(true);
        assert_eq!(control.next(false), Some(true));
        assert_eq!(control.next(false), None);
    }
}
