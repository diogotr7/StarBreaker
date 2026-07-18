//! SWF stage/sprite display-list extractors.
//!
//! Provides both standalone `extract_*(bytes)` entry points (used by
//! examples/tests) and `*_from_tags(&[Tag])` cores that `SwfAssetLibrary`
//! calls after a single parse. `extract_all_stage_frames_from_tags` snapshots
//! every main-timeline frame in one walk so `stage_frame(N)` needs no re-parse.

use std::collections::HashMap;

use swf::{CharacterId, Depth, Matrix, Tag};

use crate::error::UiError;

use super::types::PlaceRecord;

/// Walk a sprite's tag stream up to the first `ShowFrame` and return the
/// resulting display-list `PlaceRecord`s sorted by depth.
fn sprite_first_frame_records(tags: &[Tag]) -> Vec<PlaceRecord> {
    let mut depth_map: HashMap<Depth, PlaceRecord> = HashMap::new();

    'walk: for tag in tags {
        match tag {
            Tag::ShowFrame => break 'walk,
            Tag::PlaceObject(po) => {
                let previous = depth_map.get(&po.depth).cloned();
                let character_id = match po.action {
                    swf::PlaceObjectAction::Place(id) => Some(id),
                    swf::PlaceObjectAction::Replace(id) => Some(id),
                    swf::PlaceObjectAction::Modify => previous.as_ref().map(|r| r.character_id),
                };
                let Some(character_id) = character_id else {
                    continue;
                };
                depth_map.insert(
                    po.depth,
                    PlaceRecord {
                        depth: po.depth,
                        character_id,
                        matrix: po
                            .matrix
                            .or_else(|| previous.as_ref().map(|r| r.matrix))
                            .unwrap_or(Matrix::IDENTITY),
                        color_transform: po
                            .color_transform
                            .or_else(|| previous.as_ref().and_then(|r| r.color_transform)),
                        name: po.name.map(|n| n.to_string_lossy(swf::UTF_8)),
                        clip_depth: po.clip_depth.or_else(|| previous.as_ref().and_then(|r| r.clip_depth)),
                    },
                );
            }
            Tag::RemoveObject(ro) => {
                depth_map.remove(&ro.depth);
            }
            Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {}
            _ => {}
        }
    }

    let mut records: Vec<PlaceRecord> = depth_map.into_values().collect();
    records.sort_by_key(|r| r.depth);
    records
}

pub fn extract_sprite_first_frame(
    swf_bytes: &[u8],
    sprite_id: CharacterId,
) -> Result<Vec<PlaceRecord>, UiError> {
    let buf = swf::decompress_swf(std::io::Cursor::new(swf_bytes))?;
    let parsed = swf::parse_swf(&buf)?;

    let sprite = parsed
        .tags
        .iter()
        .find_map(|t| match t {
            Tag::DefineSprite(s) if s.id == sprite_id => Some(s),
            _ => None,
        })
        .ok_or_else(|| UiError::UnsupportedTag(format!("DefineSprite id={sprite_id} not found")))?;

    Ok(sprite_first_frame_records(&sprite.tags))
}

/// Parse the SWF once and return every `DefineSprite`'s first-frame display
/// list, keyed by character id.  Used to populate `SwfAssetLibrary`'s cache so
/// the recursive renderer does not re-decompress/parse the SWF per sprite node.
pub fn extract_all_sprite_first_frames(
    swf_bytes: &[u8],
) -> HashMap<CharacterId, Vec<PlaceRecord>> {
    let Ok(buf) = swf::decompress_swf(std::io::Cursor::new(swf_bytes)) else {
        return HashMap::new();
    };
    let Ok(parsed) = swf::parse_swf(&buf) else {
        return HashMap::new();
    };
    extract_all_sprite_first_frames_from_tags(&parsed.tags)
}

pub(crate) fn extract_all_sprite_first_frames_from_tags(
    tags: &[Tag],
) -> HashMap<CharacterId, Vec<PlaceRecord>> {
    let mut out = HashMap::new();
    for tag in tags {
        if let Tag::DefineSprite(s) = tag {
            out.insert(s.id, sprite_first_frame_records(&s.tags));
        }
    }
    out
}

/// Apply a `PlaceObject` to `depth_map`, byte-exactly reproducing the
/// display-list update used by every stage/sprite frame walk (same
/// `previous`/`Modify`/`Replace` inheritance and `Matrix::IDENTITY` fallback).
fn apply_place_object(depth_map: &mut HashMap<Depth, PlaceRecord>, po: &swf::PlaceObject) {
    let previous = depth_map.get(&po.depth).cloned();
    let character_id = match po.action {
        swf::PlaceObjectAction::Place(id) => Some(id),
        swf::PlaceObjectAction::Replace(id) => Some(id),
        swf::PlaceObjectAction::Modify => previous.as_ref().map(|r| r.character_id),
    };
    let Some(character_id) = character_id else {
        return;
    };
    depth_map.insert(
        po.depth,
        PlaceRecord {
            depth: po.depth,
            character_id,
            matrix: po
                .matrix
                .or_else(|| previous.as_ref().map(|r| r.matrix))
                .unwrap_or(Matrix::IDENTITY),
            color_transform: po
                .color_transform
                .or_else(|| previous.as_ref().and_then(|r| r.color_transform)),
            name: po.name.map(|n| n.to_string_lossy(swf::UTF_8)),
            clip_depth: po.clip_depth.or_else(|| previous.as_ref().and_then(|r| r.clip_depth)),
        },
    );
}

fn depth_sorted(depth_map: &HashMap<Depth, PlaceRecord>) -> Vec<PlaceRecord> {
    let mut records: Vec<PlaceRecord> = depth_map.values().cloned().collect();
    records.sort_by_key(|r| r.depth);
    records
}

/// Walk the main timeline once and return `(snapshots, tail)`:
/// `snapshots[N]` is the cumulative display list just before the `(N+1)`-th
/// `ShowFrame` (i.e. the state of frame `N`); `tail` is the final display list
/// after all tags (returned for any `frame_index >= snapshots.len()`, and for
/// an empty/ShowFrame-less timeline). Byte-exactly equivalent to the old
/// per-call `extract_stage_frame` break-on-`==`/`>` loop.
pub(crate) fn extract_all_stage_frames_from_tags(
    tags: &[Tag],
) -> (Vec<Vec<PlaceRecord>>, Vec<PlaceRecord>) {
    let mut depth_map: HashMap<Depth, PlaceRecord> = HashMap::new();
    let mut snapshots: Vec<Vec<PlaceRecord>> = Vec::new();

    for tag in tags {
        match tag {
            Tag::ShowFrame => snapshots.push(depth_sorted(&depth_map)),
            Tag::PlaceObject(po) => apply_place_object(&mut depth_map, po),
            Tag::RemoveObject(ro) => {
                depth_map.remove(&ro.depth);
            }
            _ => {}
        }
    }

    let tail = depth_sorted(&depth_map);
    (snapshots, tail)
}

pub fn extract_stage_frame(swf_bytes: &[u8], frame_index: u32) -> Vec<PlaceRecord> {
    let buf = match swf::decompress_swf(std::io::Cursor::new(swf_bytes)) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("extract_stage_frame: decompress failed: {e}");
            return vec![];
        }
    };
    let parsed = match swf::parse_swf(&buf) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("extract_stage_frame: parse failed: {e}");
            return vec![];
        }
    };

    let mut depth_map: HashMap<Depth, PlaceRecord> = HashMap::new();
    let mut current_frame: u32 = 0;

    for tag in &parsed.tags {
        if current_frame > frame_index {
            break;
        }
        match tag {
            Tag::ShowFrame => {
                if current_frame == frame_index {
                    break;
                }
                current_frame += 1;
            }
            Tag::PlaceObject(po) => {
                let previous = depth_map.get(&po.depth).cloned();
                let character_id = match po.action {
                    swf::PlaceObjectAction::Place(id) => Some(id),
                    swf::PlaceObjectAction::Replace(id) => Some(id),
                    swf::PlaceObjectAction::Modify => previous.as_ref().map(|r| r.character_id),
                };
                let Some(character_id) = character_id else {
                    continue;
                };
                depth_map.insert(
                    po.depth,
                    PlaceRecord {
                        depth: po.depth,
                        character_id,
                        matrix: po
                            .matrix
                            .or_else(|| previous.as_ref().map(|r| r.matrix))
                            .unwrap_or(Matrix::IDENTITY),
                        color_transform: po
                            .color_transform
                            .or_else(|| previous.as_ref().and_then(|r| r.color_transform)),
                        name: po.name.map(|n| n.to_string_lossy(swf::UTF_8)),
                        clip_depth: po.clip_depth.or_else(|| previous.as_ref().and_then(|r| r.clip_depth)),
                    },
                );
            }
            Tag::RemoveObject(ro) => {
                depth_map.remove(&ro.depth);
            }
            Tag::DoAction(_) | Tag::DoInitAction { .. } | Tag::DoAbc(_) | Tag::DoAbc2(_) => {}
            _ => {}
        }
    }

    let mut records: Vec<PlaceRecord> = depth_map.into_values().collect();
    records.sort_by_key(|r| r.depth);
    records
}

pub fn extract_stage_size(swf_bytes: &[u8]) -> (f32, f32) {
    let buf = match swf::decompress_swf(std::io::Cursor::new(swf_bytes)) {
        Ok(b) => b,
        Err(_) => return (0.0, 0.0),
    };
    match swf::parse_swf(&buf) {
        Ok(p) => {
            let r = p.header.stage_size();
            let w = (r.x_max - r.x_min).to_pixels() as f32;
            let h = (r.y_max - r.y_min).to_pixels() as f32;
            (w, h)
        }
        Err(_) => (0.0, 0.0),
    }
}
