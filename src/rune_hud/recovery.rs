//! A requested HUD is not proof of a visible native panel. Give AppKit time
//! to show it, try reordering once, then replace a panel that stays lost.
//! This watches native visibility, independently of microphone or paste state.

use std::time::{Duration, Instant};

pub(super) const GRACE: Duration = Duration::from_millis(300);
const HEALTHY: Duration = Duration::from_secs(1);

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Action {
    None,
    Rehome,
    Recreate,
}

#[derive(Default)]
pub(super) struct Recovery {
    missing_since: Option<Instant>,
    repaired_at: Option<Instant>,
    healthy_since: Option<Instant>,
    recreated: bool,
}

impl Recovery {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(super) fn observe(&mut self, now: Instant, wanted: bool, visible: bool) -> Action {
        if !wanted {
            self.reset();
            return Action::None;
        }
        if visible {
            self.missing_since = None;
            self.repaired_at = None;
            let healthy = *self.healthy_since.get_or_insert(now);
            // A later, separate Space change must also recover. Require a
            // stable success before allowing another replacement, so a
            // transient answer cannot turn a persistent failure into a loop.
            if now.saturating_duration_since(healthy) >= HEALTHY {
                self.recreated = false;
            }
            return Action::None;
        }
        self.healthy_since = None;
        if self.recreated {
            return Action::None;
        }
        let since = *self.missing_since.get_or_insert(now);
        if now.saturating_duration_since(since) < GRACE {
            return Action::None;
        }
        match self.repaired_at {
            None => {
                self.repaired_at = Some(now);
                Action::Rehome
            }
            Some(at) if now.saturating_duration_since(at) >= GRACE => {
                self.recreated = true;
                Action::Recreate
            }
            Some(_) => Action::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_still_elsewhere_after_rehome_is_replaced() {
        let start = Instant::now();
        let mut recovery = Recovery::default();
        assert_eq!(recovery.observe(start, true, false), Action::None);
        assert_eq!(recovery.observe(start + GRACE, true, false), Action::Rehome);
        assert_eq!(
            recovery.observe(start + 2 * GRACE, true, false),
            Action::Recreate
        );
        for step in 3..100 {
            assert_eq!(
                recovery.observe(start + step * GRACE, true, false),
                Action::None
            );
        }
    }

    #[test]
    fn a_delayed_show_or_a_successful_rehome_needs_no_replacement() {
        let start = Instant::now();
        let mut recovery = Recovery::default();
        recovery.observe(start, true, false);
        assert_eq!(
            recovery.observe(start + GRACE / 2, true, true),
            Action::None
        );
        let lost = start + 2 * GRACE;
        recovery.observe(lost, true, false);
        assert_eq!(recovery.observe(lost + GRACE, true, false), Action::Rehome);
        assert_eq!(recovery.observe(lost + 2 * GRACE, true, true), Action::None);
    }

    #[test]
    fn an_intentional_hide_cancels_recovery_and_a_new_show_gets_a_fresh_budget() {
        let start = Instant::now();
        let mut recovery = Recovery::default();
        recovery.observe(start, true, false);
        recovery.observe(start + GRACE, true, false);
        assert_eq!(
            recovery.observe(start + 2 * GRACE, false, false),
            Action::None
        );
        assert_eq!(
            recovery.observe(start + 3 * GRACE, false, false),
            Action::None
        );
        let again = start + 4 * GRACE;
        recovery.observe(again, true, false);
        assert_eq!(recovery.observe(again + GRACE, true, false), Action::Rehome);
        assert_eq!(
            recovery.observe(again + 2 * GRACE, true, false),
            Action::Recreate
        );
    }

    #[test]
    fn a_transient_native_success_does_not_restart_a_failed_replacement_loop() {
        let start = Instant::now();
        let mut recovery = Recovery::default();
        recovery.observe(start, true, false);
        recovery.observe(start + GRACE, true, false);
        recovery.observe(start + 2 * GRACE, true, false);
        recovery.observe(start + 3 * GRACE, true, true);
        for step in 4..100 {
            assert_eq!(
                recovery.observe(start + step * GRACE, true, false),
                Action::None
            );
        }
    }

    #[test]
    fn a_later_stranding_recovers_after_a_stable_native_success() {
        let start = Instant::now();
        let mut recovery = Recovery::default();
        recovery.observe(start, true, false);
        recovery.observe(start + GRACE, true, false);
        recovery.observe(start + 2 * GRACE, true, false);
        let visible = start + 3 * GRACE;
        recovery.observe(visible, true, true);
        recovery.observe(visible + HEALTHY, true, true);
        let lost = visible + 2 * HEALTHY;
        recovery.observe(lost, true, false);
        assert_eq!(recovery.observe(lost + GRACE, true, false), Action::Rehome);
        assert_eq!(
            recovery.observe(lost + 2 * GRACE, true, false),
            Action::Recreate
        );
    }
}
