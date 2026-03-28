pub mod vlq;
pub mod props;
pub mod rtpc;
pub mod node_base;
pub mod event;
pub mod action;
pub mod sound;
pub mod containers;
pub mod music;
pub mod bus;
pub mod modulator;
pub mod attenuation;
pub mod misc;

pub use event::Event;
pub use action::Action;
pub use sound::Sound;
pub use containers::{RanSeqContainer, SwitchContainer, ActorMixer, BlendContainer};
pub use music::{MusicSegment, MusicTrack, MusicSwitchContainer, MusicPlaylistContainer};
pub use bus::{AudioBus, FxBase};
pub use modulator::Modulator;
pub use attenuation::Attenuation;
pub use misc::{State, DialogueEvent, AudioDevice};

use std::collections::HashMap;

use serde::Serialize;
use starbreaker_common::SpanReader;

use crate::error::BnkError;

/// HIRC object type IDs (v128+ dispatch table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[repr(u8)]
pub enum HircObjectType {
    State = 1,
    Sound = 2,
    Action = 3,
    Event = 4,
    RanSeqContainer = 5,
    SwitchContainer = 6,
    ActorMixer = 7,
    AudioBus = 8,
    BlendContainer = 9,
    MusicSegment = 10,
    MusicTrack = 11,
    MusicSwitchContainer = 12,
    MusicPlaylistContainer = 13,
    Attenuation = 14,
    DialogueEvent = 15,
    FxShareSet = 16,
    FxCustom = 17,
    AuxBus = 18,
    LfoModulator = 19,
    EnvelopeModulator = 20,
    AudioDevice = 21,
    TimeModulator = 22,
}

impl HircObjectType {
    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(Self::State),
            2 => Some(Self::Sound),
            3 => Some(Self::Action),
            4 => Some(Self::Event),
            5 => Some(Self::RanSeqContainer),
            6 => Some(Self::SwitchContainer),
            7 => Some(Self::ActorMixer),
            8 => Some(Self::AudioBus),
            9 => Some(Self::BlendContainer),
            10 => Some(Self::MusicSegment),
            11 => Some(Self::MusicTrack),
            12 => Some(Self::MusicSwitchContainer),
            13 => Some(Self::MusicPlaylistContainer),
            14 => Some(Self::Attenuation),
            15 => Some(Self::DialogueEvent),
            16 => Some(Self::FxShareSet),
            17 => Some(Self::FxCustom),
            18 => Some(Self::AuxBus),
            19 => Some(Self::LfoModulator),
            20 => Some(Self::EnvelopeModulator),
            21 => Some(Self::AudioDevice),
            22 => Some(Self::TimeModulator),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::State => "State",
            Self::Sound => "Sound",
            Self::Action => "Action",
            Self::Event => "Event",
            Self::RanSeqContainer => "RanSeqContainer",
            Self::SwitchContainer => "SwitchContainer",
            Self::ActorMixer => "ActorMixer",
            Self::AudioBus => "AudioBus",
            Self::BlendContainer => "BlendContainer",
            Self::MusicSegment => "MusicSegment",
            Self::MusicTrack => "MusicTrack",
            Self::MusicSwitchContainer => "MusicSwitchContainer",
            Self::MusicPlaylistContainer => "MusicPlaylistContainer",
            Self::Attenuation => "Attenuation",
            Self::DialogueEvent => "DialogueEvent",
            Self::FxShareSet => "FxShareSet",
            Self::FxCustom => "FxCustom",
            Self::AuxBus => "AuxBus",
            Self::LfoModulator => "LfoModulator",
            Self::EnvelopeModulator => "EnvelopeModulator",
            Self::AudioDevice => "AudioDevice",
            Self::TimeModulator => "TimeModulator",
        }
    }
}

impl std::fmt::Display for HircObjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Source type for audio media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SoundSource {
    Embedded,
    PrefetchStream,
    Stream,
}

impl SoundSource {
    pub fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Embedded,
            1 => Self::PrefetchStream,
            _ => Self::Stream,
        }
    }
}

/// A raw HIRC entry (before type dispatch).
#[derive(Debug, Clone)]
pub struct HircEntry {
    pub type_id: u8,
    pub object_id: u32,
    pub data: Vec<u8>,
}

/// Parsed HIRC section.
#[derive(Debug, Clone)]
pub struct HircSection {
    pub entries: Vec<HircEntry>,
}

impl HircSection {
    pub fn parse(data: &[u8]) -> Result<Self, BnkError> {
        let mut reader = SpanReader::new(data);
        let count = reader.read_u32()?;

        let mut entries = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let type_id = reader.read_u8()?;
            let length = reader.read_u32()? as usize;
            let object_id = reader.read_u32()?;
            let remaining_len = length.saturating_sub(4);
            let data = reader.read_bytes(remaining_len)?.to_vec();

            entries.push(HircEntry {
                type_id,
                object_id,
                data,
            });
        }

        Ok(HircSection { entries })
    }

    pub fn type_counts(&self) -> HashMap<u8, usize> {
        let mut counts = HashMap::new();
        for entry in &self.entries {
            *counts.entry(entry.type_id).or_insert(0) += 1;
        }
        counts
    }
}

// ---------------------------------------------------------------------------
// HircObject — typed dispatch enum
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub enum HircObject {
    Sound(Sound),
    Action(Action),
    Event(Event),
    RanSeqContainer(RanSeqContainer),
    SwitchContainer(SwitchContainer),
    ActorMixer(ActorMixer),
    BlendContainer(BlendContainer),
    MusicSegment(MusicSegment),
    MusicTrack(MusicTrack),
    MusicSwitchContainer(MusicSwitchContainer),
    MusicPlaylistContainer(MusicPlaylistContainer),
    State(State),
    AudioBus(AudioBus),
    Attenuation(Attenuation),
    DialogueEvent(DialogueEvent),
    FxShareSet(FxBase),
    FxCustom(FxBase),
    AuxBus(AudioBus),
    LfoModulator(Modulator),
    EnvelopeModulator(Modulator),
    AudioDevice(AudioDevice),
    TimeModulator(Modulator),
    Unknown { type_id: u8, id: u32, data: Vec<u8> },
}

impl HircObject {
    pub fn id(&self) -> u32 {
        match self {
            Self::Sound(s) => s.id,
            Self::Action(a) => a.id,
            Self::Event(e) => e.id,
            Self::RanSeqContainer(c) => c.id,
            Self::SwitchContainer(c) => c.id,
            Self::ActorMixer(c) => c.id,
            Self::BlendContainer(c) => c.id,
            Self::MusicSegment(m) => m.id,
            Self::MusicTrack(m) => m.id,
            Self::MusicSwitchContainer(m) => m.id,
            Self::MusicPlaylistContainer(m) => m.id,
            Self::State(s) => s.id,
            Self::AudioBus(b) => b.id,
            Self::Attenuation(a) => a.id,
            Self::DialogueEvent(d) => d.id,
            Self::FxShareSet(f) => f.id,
            Self::FxCustom(f) => f.id,
            Self::AuxBus(b) => b.id,
            Self::LfoModulator(m) => m.id,
            Self::EnvelopeModulator(m) => m.id,
            Self::AudioDevice(d) => d.id,
            Self::TimeModulator(m) => m.id,
            Self::Unknown { id, .. } => *id,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Sound(_) => "Sound",
            Self::Action(_) => "Action",
            Self::Event(_) => "Event",
            Self::RanSeqContainer(_) => "RanSeqContainer",
            Self::SwitchContainer(_) => "SwitchContainer",
            Self::ActorMixer(_) => "ActorMixer",
            Self::BlendContainer(_) => "BlendContainer",
            Self::MusicSegment(_) => "MusicSegment",
            Self::MusicTrack(_) => "MusicTrack",
            Self::MusicSwitchContainer(_) => "MusicSwitchContainer",
            Self::MusicPlaylistContainer(_) => "MusicPlaylistContainer",
            Self::State(_) => "State",
            Self::AudioBus(_) => "AudioBus",
            Self::Attenuation(_) => "Attenuation",
            Self::DialogueEvent(_) => "DialogueEvent",
            Self::FxShareSet(_) => "FxShareSet",
            Self::FxCustom(_) => "FxCustom",
            Self::AuxBus(_) => "AuxBus",
            Self::LfoModulator(_) => "LfoModulator",
            Self::EnvelopeModulator(_) => "EnvelopeModulator",
            Self::AudioDevice(_) => "AudioDevice",
            Self::TimeModulator(_) => "TimeModulator",
            Self::Unknown { type_id, .. } => {
                HircObjectType::from_u8(*type_id)
                    .map(|t| t.name())
                    .unwrap_or("Unknown")
            }
        }
    }

    pub fn children(&self) -> Option<&[u32]> {
        match self {
            Self::RanSeqContainer(c) => Some(&c.children),
            Self::SwitchContainer(c) => Some(&c.children),
            Self::ActorMixer(c) => Some(&c.children),
            Self::BlendContainer(c) => Some(&c.children),
            Self::MusicSegment(m) => Some(&m.music_params.children),
            Self::MusicSwitchContainer(m) => Some(&m.trans_params.music_params.children),
            Self::MusicPlaylistContainer(m) => Some(&m.trans_params.music_params.children),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Parse dispatch
// ---------------------------------------------------------------------------

fn parse_hirc_object(entry: &HircEntry) -> HircObject {
    let mut reader = SpanReader::new(&entry.data);
    let result = match entry.type_id {
        1 => State::parse(entry.object_id, &mut reader).map(HircObject::State),
        2 => Sound::parse(entry.object_id, &mut reader).map(HircObject::Sound),
        3 => Action::parse(entry.object_id, &mut reader).map(HircObject::Action),
        4 => Event::parse(entry.object_id, &mut reader).map(HircObject::Event),
        5 => RanSeqContainer::parse(entry.object_id, &mut reader).map(HircObject::RanSeqContainer),
        6 => SwitchContainer::parse(entry.object_id, &mut reader).map(HircObject::SwitchContainer),
        7 => ActorMixer::parse(entry.object_id, &mut reader).map(HircObject::ActorMixer),
        8 => AudioBus::parse(entry.object_id, &mut reader).map(HircObject::AudioBus),
        9 => BlendContainer::parse(entry.object_id, &mut reader).map(HircObject::BlendContainer),
        10 => MusicSegment::parse(entry.object_id, &mut reader).map(HircObject::MusicSegment),
        11 => MusicTrack::parse(entry.object_id, &mut reader).map(HircObject::MusicTrack),
        12 => MusicSwitchContainer::parse(entry.object_id, &mut reader).map(HircObject::MusicSwitchContainer),
        13 => MusicPlaylistContainer::parse(entry.object_id, &mut reader).map(HircObject::MusicPlaylistContainer),
        14 => Attenuation::parse(entry.object_id, &mut reader).map(HircObject::Attenuation),
        15 => DialogueEvent::parse(entry.object_id, &mut reader).map(HircObject::DialogueEvent),
        16 => FxBase::parse(entry.object_id, &mut reader).map(HircObject::FxShareSet),
        17 => FxBase::parse(entry.object_id, &mut reader).map(HircObject::FxCustom),
        18 => AudioBus::parse(entry.object_id, &mut reader).map(HircObject::AuxBus),
        19 => Modulator::parse(entry.object_id, &mut reader).map(HircObject::LfoModulator),
        20 => Modulator::parse(entry.object_id, &mut reader).map(HircObject::EnvelopeModulator),
        21 => AudioDevice::parse(entry.object_id, &mut reader).map(HircObject::AudioDevice),
        22 => Modulator::parse(entry.object_id, &mut reader).map(HircObject::TimeModulator),
        _ => Err(starbreaker_common::ParseError::Validation {
            context: "hirc".into(),
            message: format!("unimplemented type {}", entry.type_id),
        }),
    };
    result.unwrap_or_else(|_| HircObject::Unknown {
        type_id: entry.type_id,
        id: entry.object_id,
        data: entry.data.clone(),
    })
}

// ---------------------------------------------------------------------------
// ResolvedSound
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedSound {
    pub media_id: u32,
    pub source: SoundSource,
    pub path: Vec<u32>,
}

// ---------------------------------------------------------------------------
// Hierarchy
// ---------------------------------------------------------------------------

pub struct Hierarchy {
    objects: HashMap<u32, HircObject>,
}

impl Hierarchy {
    pub fn from_section(section: &HircSection) -> Self {
        let mut objects = HashMap::with_capacity(section.entries.len());
        for entry in &section.entries {
            let obj = parse_hirc_object(entry);
            objects.insert(obj.id(), obj);
        }
        Hierarchy { objects }
    }

    pub fn get(&self, id: u32) -> Option<&HircObject> {
        self.objects.get(&id)
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Collect all media references from Sound and MusicTrack objects directly,
    /// bypassing event resolution. Useful when event chains don't resolve (e.g. music banks).
    pub fn all_media(&self) -> Vec<ResolvedSound> {
        let mut results = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for obj in self.objects.values() {
            match obj {
                HircObject::Sound(sound) => {
                    if seen.insert(sound.media_id) {
                        results.push(ResolvedSound {
                            media_id: sound.media_id,
                            source: sound.stream_type,
                            path: vec![sound.id],
                        });
                    }
                }
                HircObject::MusicTrack(track) => {
                    for source in &track.sources {
                        if seen.insert(source.media_id) {
                            results.push(ResolvedSound {
                                media_id: source.media_id,
                                source: source.stream_type,
                                path: vec![track.id],
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        results.sort_by_key(|s| s.media_id);
        results
    }

    pub fn events(&self) -> impl Iterator<Item = (u32, &Event)> {
        self.objects.iter().filter_map(|(id, obj)| {
            if let HircObject::Event(e) = obj { Some((*id, e)) } else { None }
        })
    }

    pub fn resolve_event(&self, event_id: u32) -> Vec<ResolvedSound> {
        let event = match self.get(event_id) {
            Some(HircObject::Event(e)) => e,
            _ => return Vec::new(),
        };
        let mut results = Vec::new();
        for &action_id in &event.action_ids {
            if let Some(HircObject::Action(action)) = self.get(action_id) {
                if action.is_play() {
                    let mut path = vec![event_id, action_id];
                    self.resolve_target(action.target_id, &mut path, &mut results);
                }
            }
        }
        results
    }

    pub fn resolve_event_by_name(&self, name: &str) -> Vec<ResolvedSound> {
        let hash = crate::fnv::fnv1_hash(name);
        self.resolve_event(hash)
    }

    fn resolve_target(&self, target_id: u32, path: &mut Vec<u32>, results: &mut Vec<ResolvedSound>) {
        path.push(target_id);
        match self.get(target_id) {
            Some(HircObject::Sound(sound)) => {
                if sound.is_codec() {
                    results.push(ResolvedSound {
                        media_id: sound.media_id,
                        source: sound.stream_type,
                        path: path.clone(),
                    });
                }
            }
            Some(HircObject::MusicTrack(track)) => {
                // MusicTrack is a leaf — it contains WEM source references directly
                for source in &track.sources {
                    if source.is_codec() {
                        results.push(ResolvedSound {
                            media_id: source.media_id,
                            source: source.stream_type,
                            path: path.clone(),
                        });
                    }
                }
            }
            Some(obj) => {
                if let Some(children) = obj.children() {
                    for &child_id in children {
                        self.resolve_target(child_id, path, results);
                    }
                }
            }
            None => {}
        }
        path.pop();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_simple_event() {
        let section = HircSection {
            entries: vec![
                HircEntry {
                    type_id: 4,
                    object_id: 100,
                    data: {
                        let mut d = Vec::new();
                        d.push(1u8); // VLQ count = 1
                        d.extend_from_slice(&200u32.to_le_bytes());
                        d
                    },
                },
                HircEntry {
                    type_id: 3,
                    object_id: 200,
                    data: {
                        let mut d = Vec::new();
                        d.extend_from_slice(&0x0403u16.to_le_bytes()); // Play
                        d.extend_from_slice(&300u32.to_le_bytes()); // target
                        d.push(0); // is_bus
                        d.push(0); // prop count
                        d.push(0); // ranged mod count
                        d.push(0); // fade_curve
                        d.extend_from_slice(&0u32.to_le_bytes()); // bank_id
                        d.extend_from_slice(&0u32.to_le_bytes()); // bank_type
                        d
                    },
                },
            ],
        };

        let hierarchy = Hierarchy::from_section(&section);
        assert!(matches!(hierarchy.get(100), Some(HircObject::Event(_))));
        assert!(matches!(hierarchy.get(200), Some(HircObject::Action(_))));
        // Sound 300 doesn't exist, so resolve returns empty
        let results = hierarchy.resolve_event(100);
        assert!(results.is_empty());
    }
}
