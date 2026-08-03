//! Keyframe animation clips (Blender-spirit dope sheet, SDD §10).
//!
//! A [`Clip`] is a named bundle of [`AnimTrack`]s; each track animates one
//! [`TrackProperty`] of one [`da_core::NodeId`] through a list of
//! [`Keyframe`]s sorted by time. Sampling a clip at a time yields the
//! property values for that instant; [`Clip::apply_to`] writes them into a
//! [`Scene`].
//!
//! Clips are **text ground truth** like everything else in DeadAir: they
//! serialize to RON next to the zone file as `<zone>.clips.ron` (see
//! [`clips_path`], [`save_clip`], [`load_clip`]).
//!
//! Sampling rules:
//!
//! - Before the first key / after the last key the value **clamps** to that
//!   key (no extrapolation).
//! - [`Interp::Linear`] lerps vectors and *slerps* rotations.
//! - [`Interp::Step`] holds the left key's value until the next key.
//! - [`Interp::EaseInOut`] applies a smoothstep to the blend factor.
//! - The interpolation mode of the **left** key governs the segment.
//! - Bitmask tracks never blend; they always step.

use std::path::{Path, PathBuf};

use da_core::NodeId;
use da_graph::Scene;
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Which property of a node a track drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TrackProperty {
    /// Local translation of the node's [`Transform`].
    Translation,
    /// Local rotation of the node's [`Transform`].
    Rotation,
    /// Local scale of the node's [`Transform`].
    Scale,
    /// Per-child on/off bitmask of a `Switch` node (bit *i* = child *i*).
    SwitchMask,
}

impl TrackProperty {
    /// Short label for the dope-sheet row.
    pub fn label(self) -> &'static str {
        match self {
            TrackProperty::Translation => "translation",
            TrackProperty::Rotation => "rotation",
            TrackProperty::Scale => "scale",
            TrackProperty::SwitchMask => "switch",
        }
    }

    /// The zero-ish value for this property (used when keying a node that
    /// has no transform).
    pub fn default_value(self) -> PropertyValue {
        match self {
            TrackProperty::Translation => PropertyValue::Translation(Vec3::ZERO),
            TrackProperty::Rotation => PropertyValue::Rotation(Quat::IDENTITY),
            TrackProperty::Scale => PropertyValue::Scale(Vec3::ONE),
            TrackProperty::SwitchMask => PropertyValue::SwitchMask(u32::MAX),
        }
    }
}

/// A concrete animated value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PropertyValue {
    /// Local translation.
    Translation(Vec3),
    /// Local rotation.
    Rotation(Quat),
    /// Local scale.
    Scale(Vec3),
    /// Switch child bitmask.
    SwitchMask(u32),
}

impl PropertyValue {
    /// The property this value belongs to.
    pub fn property(&self) -> TrackProperty {
        match self {
            PropertyValue::Translation(_) => TrackProperty::Translation,
            PropertyValue::Rotation(_) => TrackProperty::Rotation,
            PropertyValue::Scale(_) => TrackProperty::Scale,
            PropertyValue::SwitchMask(_) => TrackProperty::SwitchMask,
        }
    }

    /// Blend `self` → `other` by `f ∈ [0, 1]`. Vectors lerp, rotations
    /// slerp, bitmasks step at `f >= 1`. Mismatched variants return `self`.
    pub fn blend(&self, other: &PropertyValue, f: f32) -> PropertyValue {
        let f = if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 };
        match (self, other) {
            (PropertyValue::Translation(a), PropertyValue::Translation(b)) => {
                PropertyValue::Translation(a.lerp(*b, f))
            }
            (PropertyValue::Scale(a), PropertyValue::Scale(b)) => {
                PropertyValue::Scale(a.lerp(*b, f))
            }
            (PropertyValue::Rotation(a), PropertyValue::Rotation(b)) => {
                PropertyValue::Rotation(a.slerp(*b, f))
            }
            (PropertyValue::SwitchMask(a), PropertyValue::SwitchMask(b)) => {
                PropertyValue::SwitchMask(if f >= 1.0 { *b } else { *a })
            }
            _ => *self,
        }
    }
}

/// How a segment leading out of a key is interpolated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Interp {
    /// Straight lerp / slerp.
    #[default]
    Linear,
    /// Hold this key's value until the next key.
    Step,
    /// Smoothstep-eased lerp / slerp.
    EaseInOut,
}

impl Interp {
    /// Shape a normalized `0..=1` segment factor.
    pub fn shape(self, f: f32) -> f32 {
        let f = if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 };
        match self {
            Interp::Linear => f,
            Interp::Step => 0.0,
            Interp::EaseInOut => f * f * (3.0 - 2.0 * f),
        }
    }

    /// Cycle through the modes (dope-sheet right-click / button).
    pub fn next(self) -> Interp {
        match self {
            Interp::Linear => Interp::Step,
            Interp::Step => Interp::EaseInOut,
            Interp::EaseInOut => Interp::Linear,
        }
    }
}

/// One key on a track.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    /// Time in seconds from the clip start.
    pub time_sec: f32,
    /// Value held at `time_sec`.
    pub value: PropertyValue,
    /// Interpolation out of this key toward the next.
    pub interp: Interp,
}

impl Keyframe {
    /// A key at `time_sec` holding `value` with `interp`.
    pub fn new(time_sec: f32, value: PropertyValue, interp: Interp) -> Self {
        Self {
            time_sec: sane_time(time_sec),
            value,
            interp,
        }
    }
}

fn sane_time(t: f32) -> f32 {
    if t.is_finite() {
        t.max(0.0)
    } else {
        0.0
    }
}

/// One node property animated over time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimTrack {
    /// Node this track drives.
    pub node: NodeId,
    /// Property this track drives.
    pub property: TrackProperty,
    /// Keys, kept sorted by [`Keyframe::time_sec`].
    pub keys: Vec<Keyframe>,
}

impl AnimTrack {
    /// An empty track for `node`'s `property`.
    pub fn new(node: NodeId, property: TrackProperty) -> Self {
        Self {
            node,
            property,
            keys: Vec::new(),
        }
    }

    /// Sort keys by time (stable, so equal-time keys keep insert order).
    pub fn sort(&mut self) {
        self.keys
            .sort_by(|a, b| a.time_sec.total_cmp(&b.time_sec));
    }

    /// Insert a key, replacing any existing key within `EPS` of its time,
    /// and keep the track sorted. Returns the index of the key.
    pub fn insert_key(&mut self, key: Keyframe) -> usize {
        const EPS: f32 = 1e-4;
        if let Some(i) = self
            .keys
            .iter()
            .position(|k| (k.time_sec - key.time_sec).abs() < EPS)
        {
            self.keys[i] = key;
            return i;
        }
        self.keys.push(key);
        self.sort();
        self.keys
            .iter()
            .position(|k| (k.time_sec - key.time_sec).abs() < EPS)
            .unwrap_or(0)
    }

    /// Move key `index` to `time_sec`, re-sorting; returns its new index
    /// (or `None` if `index` is out of range).
    pub fn retime(&mut self, index: usize, time_sec: f32) -> Option<usize> {
        let key = *self.keys.get(index)?;
        self.keys.remove(index);
        let moved = Keyframe {
            time_sec: sane_time(time_sec),
            ..key
        };
        self.keys.push(moved);
        self.sort();
        self.keys.iter().position(|k| *k == moved)
    }

    /// Remove key `index`, returning it.
    pub fn remove_key(&mut self, index: usize) -> Option<Keyframe> {
        (index < self.keys.len()).then(|| self.keys.remove(index))
    }

    /// Last key time, or 0 for an empty track.
    pub fn end_time(&self) -> f32 {
        self.keys.last().map(|k| k.time_sec).unwrap_or(0.0)
    }

    /// Value at `time`, clamping outside the key range. `None` when the
    /// track has no keys.
    pub fn sample(&self, time: f32) -> Option<PropertyValue> {
        if self.keys.is_empty() {
            return None;
        }
        let time = if time.is_finite() { time } else { 0.0 };
        let first = self.keys.first()?;
        if time <= first.time_sec {
            return Some(first.value);
        }
        let last = self.keys.last()?;
        if time >= last.time_sec {
            return Some(last.value);
        }
        // Segment [i, i+1] containing `time`.
        let i = self
            .keys
            .partition_point(|k| k.time_sec <= time)
            .saturating_sub(1);
        let a = self.keys.get(i)?;
        let b = self.keys.get(i + 1)?;
        let span = b.time_sec - a.time_sec;
        if span <= 0.0 {
            return Some(b.value);
        }
        let f = a.interp.shape((time - a.time_sec) / span);
        Some(a.value.blend(&b.value, f))
    }
}

/// A named animation clip: a bundle of tracks over a fixed duration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// Display name.
    pub name: String,
    /// Playback length in seconds (the loop point).
    pub duration_sec: f32,
    /// Tracks, one per (node, property) pair.
    pub tracks: Vec<AnimTrack>,
}

impl Default for Clip {
    fn default() -> Self {
        Self::new("clip", 5.0)
    }
}

impl Clip {
    /// An empty clip.
    pub fn new(name: impl Into<String>, duration_sec: f32) -> Self {
        Self {
            name: name.into(),
            duration_sec: if duration_sec.is_finite() {
                duration_sec.max(0.01)
            } else {
                5.0
            },
            tracks: Vec::new(),
        }
    }

    /// Index of the track for `(node, property)`, if it exists.
    pub fn track_index(&self, node: NodeId, property: TrackProperty) -> Option<usize> {
        self.tracks
            .iter()
            .position(|t| t.node == node && t.property == property)
    }

    /// The track for `(node, property)`, creating it if needed.
    pub fn track_mut(&mut self, node: NodeId, property: TrackProperty) -> &mut AnimTrack {
        let i = match self.track_index(node, property) {
            Some(i) => i,
            None => {
                self.tracks.push(AnimTrack::new(node, property));
                self.tracks.len() - 1
            }
        };
        &mut self.tracks[i]
    }

    /// Insert a key at `time` on `(node, property)`, creating the track.
    pub fn insert_key(
        &mut self,
        node: NodeId,
        value: PropertyValue,
        time_sec: f32,
        interp: Interp,
    ) {
        let property = value.property();
        let track = self.track_mut(node, property);
        track.insert_key(Keyframe::new(time_sec, value, interp));
        let end = self.end_time();
        if end > self.duration_sec {
            self.duration_sec = end;
        }
    }

    /// Sort every track and drop empty ones.
    pub fn normalize(&mut self) {
        for t in &mut self.tracks {
            t.sort();
        }
        self.tracks.retain(|t| !t.keys.is_empty());
    }

    /// Latest key time across all tracks.
    pub fn end_time(&self) -> f32 {
        self.tracks
            .iter()
            .map(AnimTrack::end_time)
            .fold(0.0_f32, f32::max)
    }

    /// Every track's value at `time`, in track order.
    pub fn sample(&self, time: f32) -> Vec<(NodeId, PropertyValue)> {
        self.tracks
            .iter()
            .filter_map(|t| t.sample(time).map(|v| (t.node, v)))
            .collect()
    }

    /// Sample at `time` and write the values into `scene`. Missing nodes
    /// and property/kind mismatches are skipped silently (a clip can
    /// outlive a re-expansion that renumbered the graph).
    pub fn apply_to(&self, scene: &mut Scene, time: f32) {
        for (node, value) in self.sample(time) {
            apply_value(scene, node, value);
        }
    }

    /// Wrap `time` into `[0, duration_sec)` for looping playback.
    pub fn wrap(&self, time: f32) -> f32 {
        if !time.is_finite() || self.duration_sec <= 0.0 {
            return 0.0;
        }
        time.rem_euclid(self.duration_sec)
    }
}

/// Write one sampled value into the scene.
fn apply_value(scene: &mut Scene, node: NodeId, value: PropertyValue) {
    let Some(xf) = scene.transform(node).cloned() else {
        // Only switch masks are meaningful on non-transform nodes.
        if let PropertyValue::SwitchMask(mask) = value {
            apply_mask(scene, node, mask);
        }
        return;
    };
    let (t, r, sc) = match value {
        PropertyValue::Translation(t) => (t, xf.rotation(), xf.scale()),
        PropertyValue::Rotation(r) => (xf.translation(), r, xf.scale()),
        PropertyValue::Scale(sc) => (xf.translation(), xf.rotation(), sc),
        PropertyValue::SwitchMask(mask) => {
            apply_mask(scene, node, mask);
            return;
        }
    };
    let _ = scene.set_transform(node, t, r, sc);
}

fn apply_mask(scene: &mut Scene, node: NodeId, mask: u32) {
    let Some(count) = scene.switch_mask(node).map(<[bool]>::len) else {
        return;
    };
    for i in 0..count.min(32) {
        let _ = scene.set_switch(node, i, mask & (1 << i) != 0);
    }
}

/// Read the current value of `property` on `node`, for the "Key" button.
pub fn current_value(scene: &Scene, node: NodeId, property: TrackProperty) -> PropertyValue {
    match property {
        TrackProperty::SwitchMask => {
            let mask = scene
                .switch_mask(node)
                .map(|m| {
                    m.iter()
                        .take(32)
                        .enumerate()
                        .fold(0u32, |acc, (i, &on)| if on { acc | (1 << i) } else { acc })
                })
                .unwrap_or(u32::MAX);
            PropertyValue::SwitchMask(mask)
        }
        other => match (scene.transform(node), other) {
            (Some(xf), TrackProperty::Translation) => PropertyValue::Translation(xf.translation()),
            (Some(xf), TrackProperty::Rotation) => PropertyValue::Rotation(xf.rotation()),
            (Some(xf), TrackProperty::Scale) => PropertyValue::Scale(xf.scale()),
            _ => other.default_value(),
        },
    }
}

// ----------------------------------------------------------------------
// RON persistence
// ----------------------------------------------------------------------

/// The clip file that sits beside a zone: `foo.zone.ron` → `foo.clips.ron`.
pub fn clips_path(zone_path: &Path) -> PathBuf {
    let stem = zone_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.trim_end_matches(".ron").trim_end_matches(".zone"))
        .unwrap_or("zone");
    zone_path.with_file_name(format!("{stem}.clips.ron"))
}

/// Serialize a clip to pretty RON.
pub fn clip_to_ron(clip: &Clip) -> Result<String, String> {
    ron::ser::to_string_pretty(clip, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())
}

/// Parse a clip from RON.
pub fn clip_from_ron(text: &str) -> Result<Clip, String> {
    ron::from_str::<Clip>(text).map_err(|e| e.to_string())
}

/// Write `clip` to `path` as RON.
pub fn save_clip(path: &Path, clip: &Clip) -> Result<(), String> {
    let text = clip_to_ron(clip)?;
    std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Read a clip from `path`.
pub fn load_clip(path: &Path) -> Result<Clip, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    clip_from_ron(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use da_graph::{Drawable, Shape};

    fn v(x: f32) -> PropertyValue {
        PropertyValue::Translation(Vec3::new(x, 0.0, 0.0))
    }

    fn tx(pv: PropertyValue) -> Vec3 {
        match pv {
            PropertyValue::Translation(t) => t,
            other => panic!("expected translation, got {other:?}"),
        }
    }

    fn two_key_track(interp: Interp) -> AnimTrack {
        let mut t = AnimTrack::new(NodeId(1), TrackProperty::Translation);
        t.insert_key(Keyframe::new(0.0, v(0.0), interp));
        t.insert_key(Keyframe::new(2.0, v(10.0), interp));
        t
    }

    #[test]
    fn linear_interpolation_is_exact_midpoint() {
        let t = two_key_track(Interp::Linear);
        assert!((tx(t.sample(1.0).unwrap()).x - 5.0).abs() < 1e-5);
        assert!((tx(t.sample(0.5).unwrap()).x - 2.5).abs() < 1e-5);
    }

    #[test]
    fn step_holds_the_left_key() {
        let t = two_key_track(Interp::Step);
        assert_eq!(tx(t.sample(1.999).unwrap()).x, 0.0);
        assert_eq!(tx(t.sample(2.0).unwrap()).x, 10.0);
    }

    #[test]
    fn ease_in_out_is_symmetric_and_slower_at_the_ends() {
        let t = two_key_track(Interp::EaseInOut);
        let a = tx(t.sample(0.5).unwrap()).x; // f = 0.25 → 0.15625 * 10
        let mid = tx(t.sample(1.0).unwrap()).x;
        let b = tx(t.sample(1.5).unwrap()).x;
        assert!((mid - 5.0).abs() < 1e-5, "midpoint is halfway");
        assert!(a < 2.5, "eased in: {a}");
        assert!(b > 7.5, "eased out: {b}");
        assert!(((a + b) - 10.0).abs() < 1e-4, "symmetric about the middle");
    }

    #[test]
    fn quaternion_keys_slerp() {
        let mut t = AnimTrack::new(NodeId(1), TrackProperty::Rotation);
        let a = Quat::IDENTITY;
        let b = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        t.insert_key(Keyframe::new(0.0, PropertyValue::Rotation(a), Interp::Linear));
        t.insert_key(Keyframe::new(1.0, PropertyValue::Rotation(b), Interp::Linear));
        let PropertyValue::Rotation(mid) = t.sample(0.5).unwrap() else {
            panic!("expected rotation");
        };
        let expect = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        assert!(mid.abs_diff_eq(expect, 1e-5), "slerp midpoint: {mid:?}");
        // Unit length is preserved (a plain lerp would shorten it).
        assert!((mid.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn sampling_clamps_outside_the_key_range() {
        let t = two_key_track(Interp::Linear);
        assert_eq!(tx(t.sample(-100.0).unwrap()).x, 0.0);
        assert_eq!(tx(t.sample(100.0).unwrap()).x, 10.0);
    }

    #[test]
    fn empty_track_samples_to_none() {
        let t = AnimTrack::new(NodeId(1), TrackProperty::Translation);
        assert!(t.sample(0.0).is_none());
    }

    #[test]
    fn keys_pushed_out_of_order_get_sorted() {
        let mut t = AnimTrack::new(NodeId(1), TrackProperty::Translation);
        t.keys.push(Keyframe::new(3.0, v(30.0), Interp::Linear));
        t.keys.push(Keyframe::new(1.0, v(10.0), Interp::Linear));
        t.keys.push(Keyframe::new(2.0, v(20.0), Interp::Linear));
        t.sort();
        let times: Vec<f32> = t.keys.iter().map(|k| k.time_sec).collect();
        assert_eq!(times, vec![1.0, 2.0, 3.0]);
        assert!((tx(t.sample(1.5).unwrap()).x - 15.0).abs() < 1e-5);
    }

    #[test]
    fn insert_key_replaces_a_key_at_the_same_time() {
        let mut t = two_key_track(Interp::Linear);
        t.insert_key(Keyframe::new(2.0, v(99.0), Interp::Linear));
        assert_eq!(t.keys.len(), 2);
        assert_eq!(tx(t.sample(2.0).unwrap()).x, 99.0);
    }

    #[test]
    fn retiming_preserves_ordering_and_returns_the_new_index() {
        let mut t = AnimTrack::new(NodeId(1), TrackProperty::Translation);
        t.insert_key(Keyframe::new(0.0, v(0.0), Interp::Linear));
        t.insert_key(Keyframe::new(1.0, v(10.0), Interp::Linear));
        t.insert_key(Keyframe::new(2.0, v(20.0), Interp::Linear));
        // Drag the middle key past the last one.
        let new_i = t.retime(1, 5.0).expect("retimed");
        assert_eq!(new_i, 2, "moved key is now last");
        let times: Vec<f32> = t.keys.iter().map(|k| k.time_sec).collect();
        assert_eq!(times, vec![0.0, 2.0, 5.0], "still sorted");
        assert_eq!(tx(t.keys[2].value).x, 10.0, "value travels with the key");
        // Negative drags clamp to zero rather than inverting the clip.
        t.retime(2, -3.0).expect("retimed");
        assert!(t.keys.iter().all(|k| k.time_sec >= 0.0));
        assert!(t.keys.windows(2).all(|w| w[0].time_sec <= w[1].time_sec));
    }

    #[test]
    fn remove_key_drops_it() {
        let mut t = two_key_track(Interp::Linear);
        assert!(t.remove_key(0).is_some());
        assert_eq!(t.keys.len(), 1);
        assert!(t.remove_key(7).is_none());
    }

    fn scene_with_node() -> (Scene, NodeId) {
        let mut scene = Scene::new();
        let root = scene.root();
        let xf = scene.add_transform(root).expect("transform");
        let geode = scene.add_geode(xf).expect("geode");
        scene
            .add_drawable(geode, Drawable::new(Shape::Sphere { radius: 1.0 }))
            .expect("drawable");
        (scene, xf)
    }

    #[test]
    fn apply_to_moves_a_node() {
        let (mut scene, node) = scene_with_node();
        let mut clip = Clip::new("move", 2.0);
        clip.insert_key(
            node,
            PropertyValue::Translation(Vec3::ZERO),
            0.0,
            Interp::Linear,
        );
        clip.insert_key(
            node,
            PropertyValue::Translation(Vec3::new(10.0, 0.0, 0.0)),
            2.0,
            Interp::Linear,
        );

        clip.apply_to(&mut scene, 1.0);
        let t = scene.transform(node).expect("transform").translation();
        assert!((t.x - 5.0).abs() < 1e-5, "halfway: {t:?}");
        assert_eq!(scene.world_matrix(node).w_axis.truncate().x, t.x);

        clip.apply_to(&mut scene, 2.0);
        assert!((scene.transform(node).unwrap().translation().x - 10.0).abs() < 1e-5);
    }

    #[test]
    fn rotation_and_scale_tracks_do_not_clobber_translation() {
        let (mut scene, node) = scene_with_node();
        scene
            .set_translation(node, Vec3::new(1.0, 2.0, 3.0))
            .expect("set translation");
        let mut clip = Clip::new("spin", 1.0);
        clip.insert_key(
            node,
            PropertyValue::Rotation(Quat::from_rotation_y(1.0)),
            0.0,
            Interp::Linear,
        );
        clip.insert_key(
            node,
            PropertyValue::Scale(Vec3::splat(2.0)),
            0.0,
            Interp::Linear,
        );
        clip.apply_to(&mut scene, 0.0);
        let xf = scene.transform(node).expect("transform");
        assert_eq!(xf.translation(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(xf.scale(), Vec3::splat(2.0));
        assert!(xf.rotation().abs_diff_eq(Quat::from_rotation_y(1.0), 1e-6));
    }

    #[test]
    fn switch_mask_track_toggles_children_and_steps() {
        let mut scene = Scene::new();
        let sw = scene.add_switch(scene.root()).expect("switch");
        for _ in 0..3 {
            scene.add_group(sw).expect("child");
        }
        let mut clip = Clip::new("blink", 2.0);
        clip.insert_key(sw, PropertyValue::SwitchMask(0b101), 0.0, Interp::Linear);
        clip.insert_key(sw, PropertyValue::SwitchMask(0b010), 1.0, Interp::Linear);

        clip.apply_to(&mut scene, 0.4); // bitmasks never blend
        assert_eq!(scene.switch_mask(sw).unwrap(), &[true, false, true]);
        clip.apply_to(&mut scene, 1.0);
        assert_eq!(scene.switch_mask(sw).unwrap(), &[false, true, false]);
        assert_eq!(
            current_value(&scene, sw, TrackProperty::SwitchMask),
            PropertyValue::SwitchMask(0b010)
        );
    }

    #[test]
    fn apply_to_ignores_unknown_nodes() {
        let (mut scene, _) = scene_with_node();
        let mut clip = Clip::new("ghost", 1.0);
        clip.insert_key(
            NodeId(9999),
            PropertyValue::Translation(Vec3::ONE),
            0.0,
            Interp::Linear,
        );
        clip.apply_to(&mut scene, 0.0); // must not panic
    }

    #[test]
    fn clip_sample_returns_one_value_per_track() {
        let mut clip = Clip::new("multi", 1.0);
        clip.insert_key(NodeId(1), v(0.0), 0.0, Interp::Linear);
        clip.insert_key(NodeId(2), v(4.0), 0.0, Interp::Linear);
        clip.tracks.push(AnimTrack::new(NodeId(3), TrackProperty::Scale)); // empty
        let s = clip.sample(0.0);
        assert_eq!(s.len(), 2, "empty tracks contribute nothing");
    }

    #[test]
    fn keying_past_the_duration_extends_it() {
        let mut clip = Clip::new("grow", 1.0);
        clip.insert_key(NodeId(1), v(0.0), 4.0, Interp::Linear);
        assert!((clip.duration_sec - 4.0).abs() < 1e-6);
    }

    #[test]
    fn wrap_loops_the_playhead() {
        let clip = Clip::new("loop", 3.0);
        assert!((clip.wrap(3.5) - 0.5).abs() < 1e-6);
        assert!((clip.wrap(-0.5) - 2.5).abs() < 1e-6);
    }

    #[test]
    fn ron_round_trip_preserves_every_track_kind() {
        let mut clip = Clip::new("round trip", 4.0);
        clip.insert_key(NodeId(7), v(1.5), 0.0, Interp::Step);
        clip.insert_key(
            NodeId(7),
            PropertyValue::Rotation(Quat::from_rotation_z(0.75)),
            1.0,
            Interp::EaseInOut,
        );
        clip.insert_key(
            NodeId(8),
            PropertyValue::Scale(Vec3::new(1.0, 2.0, 3.0)),
            2.0,
            Interp::Linear,
        );
        clip.insert_key(NodeId(9), PropertyValue::SwitchMask(0b1011), 3.0, Interp::Step);

        let text = clip_to_ron(&clip).expect("serialize");
        let back = clip_from_ron(&text).expect("deserialize");
        assert_eq!(back, clip);
        assert!(text.contains("round trip"), "name is in the text");
    }

    #[test]
    fn clip_file_sits_next_to_the_zone() {
        let p = clips_path(Path::new("/assets/zones/home_farm.zone.ron"));
        assert_eq!(p, PathBuf::from("/assets/zones/home_farm.clips.ron"));
    }

    #[test]
    fn save_and_load_round_trip_on_disk() {
        let mut clip = Clip::new("disk", 2.0);
        clip.insert_key(NodeId(1), v(3.0), 0.5, Interp::Linear);
        let dir = std::env::temp_dir().join("da-edit-clip-test");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = clips_path(&dir.join("t.zone.ron"));
        save_clip(&path, &clip).expect("save");
        let back = load_clip(&path).expect("load");
        assert_eq!(back, clip);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn bad_ron_is_an_error_not_a_panic() {
        assert!(clip_from_ron("not a clip").is_err());
    }
}
