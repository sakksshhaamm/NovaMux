//! Deterministic, bounded, client-only idle artwork.

use std::time::{Duration, Instant};

use crate::config::{SceneStyle, Screensaver};

pub(crate) const FRAME_INTERVAL: Duration = Duration::from_millis(125);
const FRAME_MILLIS: u128 = 125;

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
            (now.duration_since(start).as_millis() / FRAME_MILLIS) as usize
        })
    }

    pub(crate) const fn is_active(&self) -> bool {
        self.active_since.is_some()
    }
}

pub(crate) fn artwork(
    mode: Screensaver,
    style: SceneStyle,
    frame: usize,
    width: u16,
    height: u16,
) -> Vec<String> {
    let lines = if width < 18 || height < 7 {
        vec![compact(mode, style)]
    } else {
        match (mode, style) {
            (Screensaver::PandaClimb, SceneStyle::Unicode) => panda_unicode(frame, height),
            (Screensaver::CatPlay, SceneStyle::Unicode) => cat_unicode(frame, width),
            (Screensaver::PandaClimb, SceneStyle::Ascii) => panda_ascii(frame, height),
            (Screensaver::CatPlay, SceneStyle::Ascii) => cat_ascii(frame, width),
            (Screensaver::Off, _) => Vec::new(),
        }
    };
    lines
        .into_iter()
        .take(usize::from(height))
        .map(|line| clip_cells(&line, usize::from(width)))
        .collect()
}

fn compact(mode: Screensaver, style: SceneStyle) -> String {
    match (mode, style) {
        (Screensaver::PandaClimb, SceneStyle::Unicode) => "🐼🎋 NovaMux".to_owned(),
        (Screensaver::CatPlay, SceneStyle::Unicode) => "🐱🧶 NovaMux".to_owned(),
        (Screensaver::PandaClimb, SceneStyle::Ascii) => "[panda] NovaMux".to_owned(),
        (Screensaver::CatPlay, SceneStyle::Ascii) => "[cat] NovaMux".to_owned(),
        (Screensaver::Off, _) => "NovaMux".to_owned(),
    }
}

fn panda_unicode(frame: usize, height: u16) -> Vec<String> {
    let travel = usize::from(height.saturating_sub(6)).max(1);
    let phase = frame % (travel * 2);
    let offset = travel.abs_diff(phase);
    let leaf = if frame % 2 == 0 { "🍃" } else { "🌿" };
    let mut lines = vec![String::new(); offset];
    lines.extend([
        format!("        {leaf}"),
        "        🎋".to_owned(),
        "     🐼 🎋".to_owned(),
        "     🐾 🎋".to_owned(),
        "        🎋".to_owned(),
        "   panda climbing".to_owned(),
    ]);
    lines
}

fn cat_unicode(frame: usize, width: u16) -> Vec<String> {
    let travel = usize::from(width.saturating_sub(18)).max(1);
    let phase = frame % (travel * 2);
    let ball_offset = if phase < travel {
        phase
    } else {
        travel * 2 - phase
    };
    let cat_offset = ball_offset.saturating_sub(5);
    vec![
        format!("{}🐱", " ".repeat(cat_offset)),
        format!("{}🐾", " ".repeat(cat_offset + 2)),
        format!("{}🧶", " ".repeat(ball_offset)),
        format!("{}cat playing", " ".repeat(cat_offset)),
    ]
}

fn panda_ascii(frame: usize, height: u16) -> Vec<String> {
    let travel = usize::from(height.saturating_sub(3)).max(1);
    let phase = frame % (travel * 2);
    let mut lines = vec![String::new(); travel.abs_diff(phase)];
    lines.extend([
        "[panda]  bamboo".to_owned(),
        " climbing upward".to_owned(),
        "      NovaMux".to_owned(),
    ]);
    lines
}

fn cat_ascii(frame: usize, width: u16) -> Vec<String> {
    let travel = usize::from(width.saturating_sub(20)).max(1);
    let phase = frame % (travel * 2);
    let offset = if phase < travel {
        phase
    } else {
        travel * 2 - phase
    };
    vec![
        format!("[cat]{}(ball)", " ".repeat(offset)),
        "       playing".to_owned(),
        "       NovaMux".to_owned(),
    ]
}

pub(crate) fn display_width(text: &str) -> usize {
    text.chars().map(cell_width).sum()
}

pub(crate) fn clip_cells(text: &str, maximum: usize) -> String {
    let mut width = 0;
    text.chars()
        .take_while(|character| {
            let next = width + cell_width(*character);
            if next > maximum {
                false
            } else {
                width = next;
                true
            }
        })
        .collect()
}

const fn cell_width(character: char) -> usize {
    let value = character as u32;
    if value == 0x200d
        || (value >= 0xfe00 && value <= 0xfe0f)
        || (value >= 0x0300 && value <= 0x036f)
    {
        0
    } else if (value >= 0x1100 && value <= 0x115f)
        || (value >= 0x2e80 && value <= 0xa4cf)
        || (value >= 0xac00 && value <= 0xd7a3)
        || (value >= 0xf900 && value <= 0xfaff)
        || (value >= 0xfe10 && value <= 0xfe6f)
        || (value >= 0xff00 && value <= 0xff60)
        || (value >= 0x1f300 && value <= 0x1faff)
    {
        2
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_frames_are_recognizable_deterministic_and_bounded() {
        let panda = artwork(Screensaver::PandaClimb, SceneStyle::Unicode, 12, 80, 24);
        assert_eq!(
            panda,
            artwork(Screensaver::PandaClimb, SceneStyle::Unicode, 12, 80, 24)
        );
        assert!(panda.iter().any(|line| line.contains("🐼")));
        assert!(panda.iter().any(|line| line.contains("🎋")));
        let cat = artwork(Screensaver::CatPlay, SceneStyle::Unicode, 999_999, 80, 24);
        assert!(cat.iter().any(|line| line.contains("🐱")));
        assert!(cat.iter().any(|line| line.contains("🧶")));
        for line in panda.iter().chain(&cat) {
            assert!(display_width(line) <= 80);
        }
    }

    #[test]
    fn display_width_and_clipping_respect_wide_cells() {
        assert_eq!(display_width("a🐼🎋"), 5);
        assert_eq!(clip_cells("a🐼🎋", 3), "a🐼");
        assert_eq!(clip_cells("🐼", 1), "");
    }

    #[test]
    fn tiny_and_ascii_fallbacks_are_deterministic() {
        assert_eq!(
            artwork(Screensaver::CatPlay, SceneStyle::Unicode, 4, 10, 4),
            vec!["🐱🧶 NovaM"]
        );
        let ascii = artwork(Screensaver::PandaClimb, SceneStyle::Ascii, 4, 80, 24);
        assert!(ascii.iter().all(|line| line.is_ascii()));
    }

    #[test]
    fn scene_rate_is_exactly_eight_frames_per_second() {
        assert_eq!(FRAME_INTERVAL, Duration::from_millis(125));
        let start = Instant::now();
        let mut state = IdleState::new(start);
        state.update(
            start + Duration::from_secs(10),
            Screensaver::CatPlay,
            10,
            true,
        );
        assert_eq!(state.frame(start + Duration::from_secs(11)), 8);
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
