use serde::Serialize;
use starbreaker_common::{ParseError, SpanReader};

use super::node_base::{parse_children, NodeBaseParams};
use super::SoundSource;

// ---------------------------------------------------------------------------
// Shared sub-structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct AkMeterInfo {
    pub grid_period: f64,
    pub grid_offset: f64,
    pub tempo: f32,
    pub time_sig_num_beats_bar: u8,
    pub time_sig_beat_value: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stinger {
    pub trigger_id: u32,
    pub segment_id: u32,
    pub sync_play_at: u32,
    pub cue_filter_hash: u32,
    pub dont_repeat_time: i32,
    pub num_segment_look_ahead: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MusicNodeParams {
    pub flags: u8,
    pub node_base: NodeBaseParams,
    pub children: Vec<u32>,
    pub meter_info: AkMeterInfo,
    pub meter_info_flag: u8,
    pub stingers: Vec<Stinger>,
}

impl MusicNodeParams {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let flags = reader.read_u8()?;
        let node_base = NodeBaseParams::parse(reader)?;
        let children = parse_children(reader)?;

        let meter_info = AkMeterInfo {
            grid_period: reader.read_f64()?,
            grid_offset: reader.read_f64()?,
            tempo: reader.read_f32()?,
            time_sig_num_beats_bar: reader.read_u8()?,
            time_sig_beat_value: reader.read_u8()?,
        };

        let meter_info_flag = reader.read_u8()?;

        let num_stingers = reader.read_u32()? as usize;
        let mut stingers = Vec::with_capacity(num_stingers);
        for _ in 0..num_stingers {
            stingers.push(Stinger {
                trigger_id: reader.read_u32()?,
                segment_id: reader.read_u32()?,
                sync_play_at: reader.read_u32()?,
                cue_filter_hash: reader.read_u32()?,
                dont_repeat_time: reader.read_i32()?,
                num_segment_look_ahead: reader.read_u32()?,
            });
        }

        Ok(MusicNodeParams {
            flags,
            node_base,
            children,
            meter_info,
            meter_info_flag,
            stingers,
        })
    }
}

// ---------------------------------------------------------------------------
// MusicTransitionObject (optional inside transition rules)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MusicTransitionObject {
    pub segment_id: u32,
    pub fade_in_time: i32,
    pub fade_in_curve: u32,
    pub fade_in_offset: i32,
    pub fade_out_time: i32,
    pub fade_out_curve: u32,
    pub fade_out_offset: i32,
    pub play_pre_entry: bool,
    pub play_post_exit: bool,
}

// ---------------------------------------------------------------------------
// MusicTransitionRule
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MusicTransitionRule {
    pub src_ids: Vec<u32>,
    pub dst_ids: Vec<u32>,
    // AkMusicTransSrcRule
    pub src_transition_time: i32,
    pub src_fade_curve: u32,
    pub src_fade_offset: i32,
    pub src_sync_type: u32,
    pub src_cue_filter_hash: u32,
    pub src_play_post_exit: bool,
    // AkMusicTransDstRule
    pub dst_transition_time: i32,
    pub dst_fade_curve: u32,
    pub dst_fade_offset: i32,
    pub dst_cue_filter_hash: u32,
    pub dst_jump_to_id: u32,
    pub dst_jump_to_type: u16,
    pub dst_entry_type: u16,
    pub dst_play_pre_entry: bool,
    pub dst_match_source_cue_name: bool,
    // Optional transition object
    pub transition_object: Option<MusicTransitionObject>,
}

// ---------------------------------------------------------------------------
// MusicTransNodeParams (used by types 12, 13)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MusicTransNodeParams {
    pub music_params: MusicNodeParams,
    pub rules: Vec<MusicTransitionRule>,
}

impl MusicTransNodeParams {
    pub fn parse(reader: &mut SpanReader) -> Result<Self, ParseError> {
        let music_params = MusicNodeParams::parse(reader)?;

        let num_rules = reader.read_u32()? as usize;
        let mut rules = Vec::with_capacity(num_rules);
        for _ in 0..num_rules {
            // Source IDs
            let num_src = reader.read_u32()? as usize;
            let mut src_ids = Vec::with_capacity(num_src);
            for _ in 0..num_src {
                src_ids.push(reader.read_u32()?);
            }

            // Destination IDs
            let num_dst = reader.read_u32()? as usize;
            let mut dst_ids = Vec::with_capacity(num_dst);
            for _ in 0..num_dst {
                dst_ids.push(reader.read_u32()?);
            }

            // AkMusicTransSrcRule
            let src_transition_time = reader.read_i32()?;
            let src_fade_curve = reader.read_u32()?;
            let src_fade_offset = reader.read_i32()?;
            let src_sync_type = reader.read_u32()?;
            let src_cue_filter_hash = reader.read_u32()?;
            let src_play_post_exit = reader.read_u8()? != 0;

            // AkMusicTransDstRule
            let dst_transition_time = reader.read_i32()?;
            let dst_fade_curve = reader.read_u32()?;
            let dst_fade_offset = reader.read_i32()?;
            let dst_cue_filter_hash = reader.read_u32()?;
            let dst_jump_to_id = reader.read_u32()?;
            let dst_jump_to_type = reader.read_u16()?;
            let dst_entry_type = reader.read_u16()?;
            let dst_play_pre_entry = reader.read_u8()? != 0;
            let dst_match_source_cue_name = reader.read_u8()? != 0;

            // AllocTransObjectFlag
            let alloc_flag = reader.read_u8()?;
            let transition_object = if alloc_flag != 0 {
                Some(MusicTransitionObject {
                    segment_id: reader.read_u32()?,
                    fade_in_time: reader.read_i32()?,
                    fade_in_curve: reader.read_u32()?,
                    fade_in_offset: reader.read_i32()?,
                    fade_out_time: reader.read_i32()?,
                    fade_out_curve: reader.read_u32()?,
                    fade_out_offset: reader.read_i32()?,
                    play_pre_entry: reader.read_u8()? != 0,
                    play_post_exit: reader.read_u8()? != 0,
                })
            } else {
                None
            };

            rules.push(MusicTransitionRule {
                src_ids,
                dst_ids,
                src_transition_time,
                src_fade_curve,
                src_fade_offset,
                src_sync_type,
                src_cue_filter_hash,
                src_play_post_exit,
                dst_transition_time,
                dst_fade_curve,
                dst_fade_offset,
                dst_cue_filter_hash,
                dst_jump_to_id,
                dst_jump_to_type,
                dst_entry_type,
                dst_play_pre_entry,
                dst_match_source_cue_name,
                transition_object,
            });
        }

        Ok(MusicTransNodeParams { music_params, rules })
    }
}

// ---------------------------------------------------------------------------
// Type 10: MusicSegment
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MusicMarker {
    pub id: u32,
    pub position: f64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MusicSegment {
    pub id: u32,
    pub music_params: MusicNodeParams,
    pub duration: f64,
    pub markers: Vec<MusicMarker>,
}

impl MusicSegment {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let music_params = MusicNodeParams::parse(reader)?;
        let duration = reader.read_f64()?;

        let num_markers = reader.read_u32()? as usize;
        let mut markers = Vec::with_capacity(num_markers);
        for _ in 0..num_markers {
            let marker_id = reader.read_u32()?;
            let position = reader.read_f64()?;

            // Null-terminated marker name (v137+)
            let mut name_bytes = Vec::new();
            loop {
                let b = reader.read_u8()?;
                if b == 0 {
                    break;
                }
                name_bytes.push(b);
            }
            let name = String::from_utf8_lossy(&name_bytes).into_owned();

            markers.push(MusicMarker {
                id: marker_id,
                position,
                name,
            });
        }

        Ok(MusicSegment {
            id,
            music_params,
            duration,
            markers,
        })
    }
}

// ---------------------------------------------------------------------------
// Type 11: MusicTrack
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct TrackSource {
    pub plugin_id: u32,
    pub stream_type: SoundSource,
    pub media_id: u32,
    pub cache_id: u32,
    pub in_memory_size: u32,
    pub source_bits: u8,
}

impl TrackSource {
    pub fn is_codec(&self) -> bool {
        (self.plugin_id & 0x0F) == 1
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackPlaylistItem {
    pub track_id: u32,
    pub source_id: u32,
    pub cache_id: u32,
    pub event_id: u32,
    pub play_at: f64,
    pub begin_trim_offset: f64,
    pub end_trim_offset: f64,
    pub src_duration: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClipAutomationPoint {
    pub from: f32,
    pub to: f32,
    pub interp: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClipAutomation {
    pub clip_index: u32,
    pub auto_type: u32,
    pub points: Vec<ClipAutomationPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrackSwitchParams {
    pub group_type: u8,
    pub group_id: u32,
    pub default_switch: u32,
    pub switch_associations: Vec<u32>,
    // TransParams
    pub src_transition_time: i32,
    pub src_fade_curve: u32,
    pub src_fade_offset: i32,
    pub sync_type: u32,
    pub cue_filter_hash: u32,
    pub dst_transition_time: i32,
    pub dst_fade_curve: u32,
    pub dst_fade_offset: i32,
}

#[derive(Debug, Clone, Serialize)]
pub struct MusicTrack {
    pub id: u32,
    pub sources: Vec<TrackSource>,
    pub flags: u8,
    pub playlist: Vec<TrackPlaylistItem>,
    pub num_sub_track: Option<u32>,
    pub clip_automations: Vec<ClipAutomation>,
    pub node_base: NodeBaseParams,
    pub track_type: u8,
    pub switch_params: Option<TrackSwitchParams>,
    pub look_ahead_time: i32,
}

impl MusicTrack {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        // v154: sources come BEFORE uFlags
        let num_sources = reader.read_u32()? as usize;
        let mut sources = Vec::with_capacity(num_sources);
        for _ in 0..num_sources {
            let plugin_id = reader.read_u32()?;
            let plugin_type = plugin_id & 0x0F;
            let stream_type = SoundSource::from_u8(reader.read_u8()?);
            let media_id = reader.read_u32()?;
            let cache_id = reader.read_u32()?;
            let in_memory_size = reader.read_u32()?;
            let source_bits = reader.read_u8()?;

            if plugin_type == 2 {
                let param_size = reader.read_u32()? as usize;
                reader.advance(param_size)?;
            }

            sources.push(TrackSource {
                plugin_id,
                stream_type,
                media_id,
                cache_id,
                in_memory_size,
                source_bits,
            });
        }

        // v153+: uFlags is after sources
        let flags = reader.read_u8()?;

        // Playlist
        let num_playlist = reader.read_u32()? as usize;
        let mut playlist = Vec::with_capacity(num_playlist);
        let num_sub_track = if num_playlist > 0 {
            for _ in 0..num_playlist {
                playlist.push(TrackPlaylistItem {
                    track_id: reader.read_u32()?,
                    source_id: reader.read_u32()?,
                    cache_id: reader.read_u32()?,
                    event_id: reader.read_u32()?,
                    play_at: reader.read_f64()?,
                    begin_trim_offset: reader.read_f64()?,
                    end_trim_offset: reader.read_f64()?,
                    src_duration: reader.read_f64()?,
                });
            }
            Some(reader.read_u32()?)
        } else {
            None
        };

        // Clip automations
        let num_clip_auto = reader.read_u32()? as usize;
        let mut clip_automations = Vec::with_capacity(num_clip_auto);
        for _ in 0..num_clip_auto {
            let clip_index = reader.read_u32()?;
            let auto_type = reader.read_u32()?;
            let num_points = reader.read_u32()? as usize;
            let mut points = Vec::with_capacity(num_points);
            for _ in 0..num_points {
                points.push(ClipAutomationPoint {
                    from: reader.read_f32()?,
                    to: reader.read_f32()?,
                    interp: reader.read_u32()?,
                });
            }
            clip_automations.push(ClipAutomation {
                clip_index,
                auto_type,
                points,
            });
        }

        // NodeBaseParams (NOT MusicNodeParams)
        let node_base = NodeBaseParams::parse(reader)?;

        // Track type
        let track_type = reader.read_u8()?;
        let switch_params = if track_type == 3 {
            // Switch track
            let group_type = reader.read_u8()?;
            let group_id = reader.read_u32()?;
            let default_switch = reader.read_u32()?;
            let num_assoc = reader.read_u32()? as usize;
            let mut switch_associations = Vec::with_capacity(num_assoc);
            for _ in 0..num_assoc {
                switch_associations.push(reader.read_u32()?);
            }
            // TransParams
            let src_transition_time = reader.read_i32()?;
            let src_fade_curve = reader.read_u32()?;
            let src_fade_offset = reader.read_i32()?;
            let sync_type = reader.read_u32()?;
            let cue_filter_hash = reader.read_u32()?;
            let dst_transition_time = reader.read_i32()?;
            let dst_fade_curve = reader.read_u32()?;
            let dst_fade_offset = reader.read_i32()?;

            Some(TrackSwitchParams {
                group_type,
                group_id,
                default_switch,
                switch_associations,
                src_transition_time,
                src_fade_curve,
                src_fade_offset,
                sync_type,
                cue_filter_hash,
                dst_transition_time,
                dst_fade_curve,
                dst_fade_offset,
            })
        } else {
            None
        };

        let look_ahead_time = reader.read_i32()?;

        Ok(MusicTrack {
            id,
            sources,
            flags,
            playlist,
            num_sub_track,
            clip_automations,
            node_base,
            track_type,
            switch_params,
            look_ahead_time,
        })
    }
}

// ---------------------------------------------------------------------------
// Type 12: MusicSwitchContainer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MusicSwitchContainer {
    pub id: u32,
    pub trans_params: MusicTransNodeParams,
    pub is_continue_playback: bool,
    pub tree_depth: u32,
    pub group_ids: Vec<u32>,
    pub group_types: Vec<u8>,
    pub tree_data_size: u32,
    pub tree_mode: u8,
    pub decision_tree: Vec<u8>,
}

impl MusicSwitchContainer {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let trans_params = MusicTransNodeParams::parse(reader)?;
        let is_continue_playback = reader.read_u8()? != 0;

        let tree_depth = reader.read_u32()?;
        let mut group_ids = Vec::with_capacity(tree_depth as usize);
        for _ in 0..tree_depth {
            group_ids.push(reader.read_u32()?);
        }
        let mut group_types = Vec::with_capacity(tree_depth as usize);
        for _ in 0..tree_depth {
            group_types.push(reader.read_u8()?);
        }

        let tree_data_size = reader.read_u32()?;
        let tree_mode = reader.read_u8()?;

        // Decision tree: store as raw bytes
        let decision_tree = reader.read_bytes(tree_data_size as usize)?.to_vec();

        Ok(MusicSwitchContainer {
            id,
            trans_params,
            is_continue_playback,
            tree_depth,
            group_ids,
            group_types,
            tree_data_size,
            tree_mode,
            decision_tree,
        })
    }
}

// ---------------------------------------------------------------------------
// Type 13: MusicPlaylistContainer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MusicPlaylistItem {
    pub segment_id: u32,
    pub playlist_item_id: u32,
    pub num_children: u32,
    pub rs_type: u32,
    pub loop_count: i16,
    pub loop_min: i16,
    pub loop_max: i16,
    pub weight: u32,
    pub avoid_repeat_count: u16,
    pub is_using_weight: bool,
    pub is_shuffle: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MusicPlaylistContainer {
    pub id: u32,
    pub trans_params: MusicTransNodeParams,
    pub playlist_items: Vec<MusicPlaylistItem>,
}

impl MusicPlaylistContainer {
    pub fn parse(id: u32, reader: &mut SpanReader) -> Result<Self, ParseError> {
        let trans_params = MusicTransNodeParams::parse(reader)?;

        let num_items = reader.read_u32()? as usize;
        let mut playlist_items = Vec::with_capacity(num_items);
        for _ in 0..num_items {
            playlist_items.push(MusicPlaylistItem {
                segment_id: reader.read_u32()?,
                playlist_item_id: reader.read_u32()?,
                num_children: reader.read_u32()?,
                rs_type: reader.read_u32()?,
                loop_count: reader.read_i16()?,
                loop_min: reader.read_i16()?,
                loop_max: reader.read_i16()?,
                weight: reader.read_u32()?,
                avoid_repeat_count: reader.read_u16()?,
                is_using_weight: reader.read_u8()? != 0,
                is_shuffle: reader.read_u8()? != 0,
            });
        }

        Ok(MusicPlaylistContainer {
            id,
            trans_params,
            playlist_items,
        })
    }
}
