//! Local VALORANT event tracking. Detection confidence and clip selection are
//! separate: repeated exact observations can save a clip; uncertain readings
//! remain reviewable. No round, ace or headshot is inferred from a kill count.
use serde::{Deserialize, Serialize};

pub const MAX_CLIP_SECONDS: f64 = 60.0;
pub const RETAIN_SECONDS: u32 = 120;
pub const MAX_LAG_SECONDS: f64 = 40.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub enabled: bool,
    pub min_kills: u32,
    pub before: u32,
    pub after: u32,
    pub merge_gap: u32,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            enabled: false,
            min_kills: 2,
            before: 12,
            after: 6,
            merge_gap: 8,
        }
    }
}
impl Preferences {
    pub fn sanitized(mut self) -> Self {
        self.min_kills = self.min_kills.clamp(1, 5);
        self.before = self.before.clamp(3, 25);
        self.after = self.after.clamp(2, 15);
        self.merge_gap = self.merge_gap.clamp(3, 15);
        self
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub victim: String,
    pub text: String,
    pub exact: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Confirmed,
    Review,
}

#[derive(Clone, Debug, Serialize)]
pub struct Kill {
    pub at: f64,
    pub text: String,
    pub confidence: Confidence,
}

#[derive(Clone, Debug, Serialize)]
pub struct Moment {
    pub start: f64,
    pub end: f64,
    pub events: Vec<Kill>,
}
impl Moment {
    pub fn confirmed_kills(&self) -> usize {
        self.events
            .iter()
            .filter(|e| e.confidence == Confidence::Confirmed)
            .count()
    }
    pub fn label(&self) -> String {
        format!("{}K", self.events.len())
    }
}

pub fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}
fn confusable(value: &str) -> String {
    normalized(value)
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' | 'l' => 'i',
            _ => c,
        })
        .collect()
}
/// Limited substitutions can associate observations, but cannot on their own
/// produce a confirmed event. Never accept arbitrary edit distance for a killer.
pub fn name_matches(read: &str, player: &str) -> Option<bool> {
    let read = normalized(read);
    let player = normalized(player);
    if read == player {
        Some(true)
    } else if player.len() >= 5
        && read.len() == player.len()
        && confusable(&read) == confusable(&player)
        && read
            .bytes()
            .zip(player.bytes())
            .filter(|(a, b)| a != b)
            .count()
            == 1
    {
        Some(false)
    } else {
        None
    }
}
fn same_victim(a: &str, b: &str) -> bool {
    let a = confusable(a);
    let b = confusable(b);
    a == b
        || (a.len() >= 6
            && a.len() == b.len()
            && a.bytes().zip(b.bytes()).filter(|(a, b)| a != b).count() == 1)
}

struct Track {
    victim: String,
    first: f64,
    last: f64,
    hits: u32,
    exact: u32,
    emitted: bool,
}

#[derive(Default)]
pub struct Tracker {
    tracks: Vec<Track>,
    last_frame: Option<f64>,
    initialized: bool,
    suppress_initial: bool,
}
impl Tracker {
    pub fn live() -> Self {
        Self {
            suppress_initial: true,
            ..Self::default()
        }
    }
    pub fn reset_after_gap(&mut self) {
        self.tracks.clear();
        self.last_frame = None;
        self.initialized = false;
    }
    pub fn frame(&mut self, at: f64, rows: &[Row]) -> Vec<Kill> {
        if !at.is_finite() || at < 0.0 || self.last_frame.is_some_and(|last| at <= last + 0.01) {
            return Vec::new();
        }
        if self.last_frame.is_some_and(|last| at - last > 2.0) {
            self.reset_after_gap();
        }
        self.last_frame = Some(at);
        self.tracks.retain(|track| at - track.last <= 8.0);
        let seed = self.suppress_initial && !self.initialized;
        self.initialized = true;
        let mut used = Vec::new();
        let mut frame_names = Vec::new();
        let mut events = Vec::new();
        for row in rows.iter().take(12) {
            let key = normalized(&row.victim);
            if frame_names.contains(&key) {
                continue;
            }
            frame_names.push(key);
            let index = self
                .tracks
                .iter()
                .enumerate()
                .find(|(i, t)| !used.contains(i) && same_victim(&t.victim, &row.victim))
                .map(|(i, _)| i)
                .unwrap_or_else(|| {
                    self.tracks.push(Track {
                        victim: row.victim.clone(),
                        first: at,
                        last: at,
                        hits: 0,
                        exact: 0,
                        emitted: seed,
                    });
                    self.tracks.len() - 1
                });
            used.push(index);
            let track = &mut self.tracks[index];
            if !track.emitted && at - track.last > 1.25 {
                track.first = at;
                track.hits = 0;
                track.exact = 0;
            }
            track.last = at;
            track.hits += 1;
            track.exact += u32::from(row.exact);
            if !track.emitted && track.hits >= 2 && track.exact >= 2 {
                track.emitted = true;
                events.push(Kill {
                    at: track.first,
                    text: row.text.chars().take(160).collect(),
                    confidence: Confidence::Confirmed,
                });
            } else if !track.emitted && track.hits >= 4 && track.exact < 2 {
                // A consistently ambiguous name is useful for manual review,
                // but must not trigger an automatic clip.
                track.emitted = true;
                events.push(Kill {
                    at: track.first,
                    text: row.text.chars().take(160).collect(),
                    confidence: Confidence::Review,
                });
            }
        }
        events
    }
}

#[derive(Default)]
pub struct Rounds {
    score: Option<(u32, u32)>,
    candidate: Option<((u32, u32), u32)>,
    banner_hits: u32,
    banner_active: bool,
    last_frame: Option<f64>,
}
impl Rounds {
    pub fn frame(&mut self, at: f64, score: Option<(u32, u32)>, banner: bool) -> bool {
        if !at.is_finite() || self.last_frame.is_some_and(|last| at <= last) {
            return false;
        }
        if self.last_frame.is_some_and(|last| at - last > 2.0) {
            self.candidate = None;
            self.banner_hits = 0;
        }
        self.last_frame = Some(at);
        let mut changed = false;
        if let Some(score) = score.filter(|(a, b)| *a <= 99 && *b <= 99) {
            let hits = match self.candidate {
                Some((s, n)) if s == score => n + 1,
                _ => 1,
            };
            self.candidate = Some((score, hits));
            if hits >= 2 && self.score != Some(score) {
                changed = self.score.is_some_and(|old| {
                    (score.0 == old.0 + 1 && score.1 == old.1)
                        || (score.1 == old.1 + 1 && score.0 == old.0)
                        || (score == (0, 0) && old != (0, 0))
                });
                if self.score.is_none() || changed {
                    self.score = Some(score);
                }
            }
        } else {
            self.candidate = None;
        }
        if banner {
            self.banner_hits += 1;
            if self.banner_hits == 2 && !self.banner_active {
                self.banner_active = true;
                changed = true;
            }
        } else {
            self.banner_hits = 0;
            self.banner_active = false;
        }
        changed
    }
}

pub struct Moments {
    preferences: Preferences,
    pending: Option<Moment>,
    ready: Vec<Moment>,
}
impl Moments {
    pub fn new(preferences: Preferences) -> Self {
        Self {
            preferences: preferences.sanitized(),
            pending: None,
            ready: Vec::new(),
        }
    }
    pub fn event(&mut self, event: Kill) {
        if !event.at.is_finite() {
            return;
        }
        let before = self.preferences.before as f64;
        let after = self.preferences.after as f64;
        let can_join = self.pending.as_ref().is_some_and(|p| {
            let last = p.events.last().unwrap().at;
            p.events.len() < 20
                && event.at >= last
                && event.at - last <= self.preferences.merge_gap as f64
                && event.at + after - p.start <= MAX_CLIP_SECONDS
        });
        if !can_join {
            self.finish();
        }
        let pending = self.pending.get_or_insert_with(|| Moment {
            start: (event.at - before).max(0.0),
            end: event.at + after,
            events: Vec::new(),
        });
        if pending.events.len() < 20 {
            pending.end = event.at + after;
            pending.events.push(event);
        }
    }
    pub fn boundary(&mut self) {
        self.finish();
    }
    pub fn advance(&mut self, through: f64) {
        if self.pending.as_ref().is_some_and(|p| {
            through
                >= p.events.last().unwrap().at
                    + self.preferences.after.max(self.preferences.merge_gap) as f64
                    + 1.0
        }) {
            self.finish();
        }
    }
    pub fn finish(&mut self) {
        if let Some(moment) = self.pending.take() {
            self.ready.push(moment);
        }
    }
    pub fn take_ready(&mut self, available_end: f64, finalizing: bool) -> Vec<Moment> {
        if finalizing {
            self.finish();
        }
        let mut output = Vec::new();
        self.ready.retain(|moment| {
            if moment.end <= available_end || finalizing {
                let mut moment = moment.clone();
                moment.end = moment.end.min(available_end);
                if moment.end - moment.start >= 0.5 {
                    output.push(moment);
                }
                false
            } else {
                true
            }
        });
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(victim: &str, exact: bool) -> Row {
        Row {
            victim: victim.into(),
            text: format!("Elvin → {victim}"),
            exact,
        }
    }
    fn kill(at: f64) -> Kill {
        Kill {
            at,
            text: "Elvin → Opponent".into(),
            confidence: Confidence::Confirmed,
        }
    }
    #[test]
    fn independent_frames_and_exact_names_are_required_for_autosaving() {
        let mut tracker = Tracker::default();
        assert!(tracker
            .frame(1.0, &[row("Opponent", true), row("Opponent", true)])
            .is_empty());
        assert!(tracker.frame(1.0, &[row("Opponent", true)]).is_empty());
        let events = tracker.frame(1.5, &[row("Opp0nent", true)]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].at, 1.0);
        assert_eq!(events[0].confidence, Confidence::Confirmed);
        assert!(tracker.frame(2.0, &[row("Opponant", true)]).is_empty());
        assert!(tracker.frame(2.5, &[]).is_empty());
        assert!(tracker.frame(3.0, &[row("Opponent", true)]).is_empty());
        assert_eq!(name_matches("E1vin", "Elvin"), Some(false));
        assert_eq!(name_matches("Elvin2", "Elvin"), None);
        assert_eq!(name_matches("Bob", "B0b"), None);
    }
    #[test]
    fn ambiguous_names_remain_review_only_and_live_start_does_not_repeat_old_feed() {
        let mut tracker = Tracker::default();
        for at in [0.0, 0.5, 1.0] {
            assert!(tracker.frame(at, &[row("Enemy", false)]).is_empty());
        }
        assert_eq!(
            tracker.frame(1.5, &[row("Enemy", false)])[0].confidence,
            Confidence::Review
        );
        let mut live = Tracker::live();
        assert!(live.frame(0.0, &[row("Old", true)]).is_empty());
        assert!(live
            .frame(0.5, &[row("Old", true), row("New", true)])
            .is_empty());
        let events = live.frame(1.0, &[row("Old", true), row("New", true)]);
        assert_eq!(events.len(), 1);
        assert!(events[0].text.contains("New"));
        // Two similar victims visible together stay distinct.
        let mut tracker = Tracker::default();
        let rows = [row("Enemy111", true), row("Enemy112", true)];
        tracker.frame(2.0, &rows);
        assert_eq!(tracker.frame(2.5, &rows).len(), 2);
    }
    #[test]
    fn multikills_merge_but_round_boundaries_and_long_gaps_split_them() {
        let mut moments = Moments::new(Preferences::default());
        for at in [20.0, 23.0, 28.0] {
            moments.event(kill(at));
        }
        moments.advance(37.0);
        let clips = moments.take_ready(37.0, false);
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].label(), "3K");
        assert_eq!((clips[0].start, clips[0].end), (8.0, 34.0));
        moments.event(kill(40.0));
        moments.boundary();
        moments.event(kill(44.0));
        assert_eq!(moments.take_ready(47.0, true).len(), 2);
        for at in (60..200).step_by(3) {
            moments.event(kill(at as f64));
        }
        assert!(moments
            .take_ready(205.0, true)
            .iter()
            .all(|m| m.end - m.start <= MAX_CLIP_SECONDS));
    }
    #[test]
    fn round_changes_need_stable_evidence_and_initial_score_is_not_a_new_round() {
        let mut rounds = Rounds::default();
        assert!(!rounds.frame(0.0, Some((2, 3)), false));
        assert!(!rounds.frame(0.5, Some((2, 3)), false));
        assert!(!rounds.frame(1.0, Some((9, 3)), false));
        assert!(!rounds.frame(1.5, Some((3, 3)), false));
        assert!(rounds.frame(2.0, Some((3, 3)), false));
        assert!(!rounds.frame(2.5, Some((3, 3)), false));
        assert!(!rounds.frame(3.0, None, true));
        assert!(rounds.frame(3.5, None, true));
        assert!(!rounds.frame(4.0, None, true));
    }
}
