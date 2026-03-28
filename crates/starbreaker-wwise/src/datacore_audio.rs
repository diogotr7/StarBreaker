//! Extract `audioTrigger` string properties from DataCore entity records.
//!
//! Uses the [`ExportSink`] trait to walk a record's object graph and capture
//! every `audioTrigger` field, regardless of schema. Works for weapons, ships,
//! items — anything with audio trigger properties.

use serde::Serialize;

use starbreaker_datacore::database::Database;
use starbreaker_datacore::error::ExportError;
use starbreaker_datacore::sink::ExportSink;
use starbreaker_datacore::types::CigGuid;
use starbreaker_datacore::walker::walk_record;

// ─── Public types ───────────────────────────────────────────────────────────

/// A single audio trigger reference found within a record.
#[derive(Debug, Clone, Serialize)]
pub struct AudioTriggerRef {
    /// The Wwise trigger name (e.g. "Play_weapon_fire").
    pub trigger_name: String,
    /// Dot-separated path to the property within the record tree.
    pub property_path: String,
}

/// All audio triggers found for a single entity record.
#[derive(Debug, Clone, Serialize)]
pub struct EntityAudioInfo {
    /// Human-readable record name.
    pub entity_name: String,
    /// File path of the record in the DataCore database.
    pub record_path: String,
    /// Collected audio trigger references.
    pub triggers: Vec<AudioTriggerRef>,
}

// ─── ExportSink implementation ──────────────────────────────────────────────

/// A sink that captures `audioTrigger` string values while ignoring everything else.
struct AudioTriggerSink {
    /// Stack of path segments. Pushed on begin_object/begin_array, popped on end.
    path_stack: Vec<String>,
    /// Collected triggers.
    triggers: Vec<AudioTriggerRef>,
}

impl AudioTriggerSink {
    fn new() -> Self {
        Self {
            path_stack: Vec::new(),
            triggers: Vec::new(),
        }
    }

    /// Build the current dot-separated property path.
    fn current_path(&self) -> String {
        self.path_stack.join(".")
    }
}

impl ExportSink for AudioTriggerSink {
    type Error = ExportError;

    fn begin_object(&mut self, name: Option<&str>) -> Result<(), Self::Error> {
        let segment = name.unwrap_or("{}").to_string();
        self.path_stack.push(segment);
        Ok(())
    }

    fn end_object(&mut self) -> Result<(), Self::Error> {
        self.path_stack.pop();
        Ok(())
    }

    fn begin_array(&mut self, name: &str) -> Result<(), Self::Error> {
        self.path_stack.push(format!("{}[]", name));
        Ok(())
    }

    fn end_array(&mut self) -> Result<(), Self::Error> {
        self.path_stack.pop();
        Ok(())
    }

    fn write_str(&mut self, name: Option<&str>, value: &str) -> Result<(), Self::Error> {
        if name == Some("audioTrigger") && !value.is_empty() {
            let mut path = self.current_path();
            if !path.is_empty() {
                path.push('.');
            }
            path.push_str("audioTrigger");

            self.triggers.push(AudioTriggerRef {
                trigger_name: value.to_string(),
                property_path: path,
            });
        }
        Ok(())
    }

    fn write_null(&mut self, _name: Option<&str>) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_bool(&mut self, _name: Option<&str>, _value: bool) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_i8(&mut self, _name: Option<&str>, _value: i8) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_i16(&mut self, _name: Option<&str>, _value: i16) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_i32(&mut self, _name: Option<&str>, _value: i32) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_i64(&mut self, _name: Option<&str>, _value: i64) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_u8(&mut self, _name: Option<&str>, _value: u8) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_u16(&mut self, _name: Option<&str>, _value: u16) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_u32(&mut self, _name: Option<&str>, _value: u32) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_u64(&mut self, _name: Option<&str>, _value: u64) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_f32(&mut self, _name: Option<&str>, _value: f32) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_f64(&mut self, _name: Option<&str>, _value: f64) -> Result<(), Self::Error> {
        Ok(())
    }

    fn write_guid(&mut self, _name: Option<&str>, _value: &CigGuid) -> Result<(), Self::Error> {
        Ok(())
    }
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Find all audio triggers in a specific record by exact name.
///
/// Returns `None` if no record with the given name exists.
pub fn find_audio_triggers(db: &Database, record_name: &str) -> Option<EntityAudioInfo> {
    let record = db
        .records()
        .iter()
        .find(|r| db.resolve_string2(r.name_offset) == record_name)?;

    let entity_name = db.resolve_string2(record.name_offset).to_string();
    let record_path = db.resolve_string(record.file_name_offset).to_string();

    let mut sink = AudioTriggerSink::new();
    // If walk fails, return the entity with no triggers rather than propagating
    // an error from an unrelated structural issue in the record.
    if walk_record(db, record, &mut sink).is_err() {
        return Some(EntityAudioInfo {
            entity_name,
            record_path,
            triggers: Vec::new(),
        });
    }

    Some(EntityAudioInfo {
        entity_name,
        record_path,
        triggers: sink.triggers,
    })
}

/// Search for entities whose record name contains `query` (case-insensitive).
///
/// Returns only entities that have at least one `audioTrigger` property.
pub fn search_entities_with_audio(db: &Database, query: &str) -> Vec<EntityAudioInfo> {
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();

    for record in db.records() {
        let name = db.resolve_string2(record.name_offset);
        if !name.to_lowercase().contains(&query_lower) {
            continue;
        }

        let entity_name = name.to_string();
        let record_path = db.resolve_string(record.file_name_offset).to_string();

        let mut sink = AudioTriggerSink::new();
        if walk_record(db, record, &mut sink).is_err() {
            continue;
        }

        if !sink.triggers.is_empty() {
            results.push(EntityAudioInfo {
                entity_name,
                record_path,
                triggers: sink.triggers,
            });
        }
    }

    results
}
