//! Deterministic, bounded, client-only idle artwork.

use std::time::{Duration, Instant};

use crate::config::Screensaver;

pub(crate) const FRAME_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) struct IdleState {
    last_input: Instant,
    active_since: Option<Instant>,
}

impl IdleState {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            last_input: now,
            active_since: None,
        }
    }

    pub(crate) fn update(
        &mut self,
        now: Instant,
        mode: Screensaver,
        idle_seconds: u16,
        allowed: bool,
    ) -> bool {
        if mode == Screensaver::Off || !allowed {
            self.active_since = None;
            return false;
        }
        if self.active_since.is_none()
            && now.duration_since(self.last_input) >= Duration::from_secs(u64::from(idle_seconds))
        {
            self.active_since = Some(now);
        }
        self.active_since.is_some()
    }

    /// Dismisses active artwork and consumes that input.
    pub(crate) fn input(&mut self, now: Instant) -> bool {
        let consumed = self.active_since.take().is_some();
        self.last_input = now;
        consumed
    }

    pub(crate) fn frame(&self, now: Instant) -> usize {
        self.active_since.map_or(0, |start| {
            (now.duration_since(start).as_millis() / 250) as usize
        })
    }

    pub(crate) const fn is_active(&self) -> bool {
        self.active_since.is_some()
    }
}

pub(crate) fn artwork(mode: Screensaver, frame: usize, width: u16, height: u16) -> Vec<String> {
    if width < 18 || height < 7 {
        return vec![match mode {
            Screensaver::PandaClimb => "ʕ•ᴥ•ʔ  NovaMux".to_owned(),
            Screensaver::CatPlay => "=^.^=  NovaMux".to_owned(),
            Screensaver::Off => "NovaMux".to_owned(),
        }];
    }
    match mode {
        Screensaver::PandaClimb => panda(frame, height),
        Screensaver::CatPlay => cat(frame, width),
        Screensaver::Off => Vec::new(),
    }
}

fn panda(frame: usize, height: u16) -> Vec<String> {
    let travel = usize::from(height.saturating_sub(6)).max(1);
    let phase = frame % (travel * 2);
    let offset = travel.abs_diff(phase);
    let grip = if frame % 2 == 0 { "/" } else { "\\" };
    let mut lines = vec![String::new(); offset];
    lines.extend([
        format!("       {grip}  ||"),
        "    ʕ •ᴥ•ʔ||".to_owned(),
        format!("    /|   |{grip}|"),
        "     |___| ||".to_owned(),
        "      / \\  ||".to_owned(),
        "   climbing quietly".to_owned(),
    ]);
    lines
}

fn cat(frame: usize, width: u16) -> Vec<String> {
    let travel = usize::from(width.saturating_sub(17)).max(1);
    let phase = frame % (travel * 2);
    let offset = if phase < travel {
        phase
    } else {
        travel * 2 - phase
    };
    let paw = if frame % 2 == 0 { "/" } else { "\\" };
    let pad = " ".repeat(offset);
    vec![
        format!("{pad} /\\_/\\\\"),
        format!("{pad}( o.o ) {paw}  o"),
        format!("{pad} > ^ <       "),
        format!("{pad}cat-play"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_deterministic_and_bounded() {
        let a = artwork(Screensaver::PandaClimb, 12, 80, 24);
        assert_eq!(a, artwork(Screensaver::PandaClimb, 12, 80, 24));
        assert!(a.len() <= 24);
        assert!(a.iter().all(|line| line.chars().count() <= 80));
        let cat = artwork(Screensaver::CatPlay, 999_999, 80, 24);
        assert!(cat.len() <= 24);
        assert!(cat.iter().all(|line| line.chars().count() <= 80));
    }

    #[test]
    fn tiny_terminals_use_compact_fallback() {
        assert_eq!(
            artwork(Screensaver::CatPlay, 4, 10, 4),
            vec!["=^.^=  NovaMux"]
        );
    }

    #[test]
    fn idle_transition_and_input_dismissal_are_explicit() {
        let start = Instant::now();
        let mut state = IdleState::new(start);
        assert!(!state.update(start, Screensaver::CatPlay, 10, true));
        let idle = start + Duration::from_secs(10);
        assert!(state.update(idle, Screensaver::CatPlay, 10, true));
        assert!(state.input(idle + Duration::from_millis(1)));
        assert!(!state.input(idle + Duration::from_millis(2)));
        assert!(!state.update(
            idle + Duration::from_secs(20),
            Screensaver::CatPlay,
            10,
            false
        ));
    }
}
