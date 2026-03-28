use crate::database::Database;
use crate::enums::{ConversionType, DataType};
use crate::error::QueryError;
use crate::query::from_datacore::FromDataCore;
use crate::query::path::parse_path;
use crate::query::value::Value;

/// A single resolved segment in a compiled path.
#[derive(Debug, Clone)]
pub struct ResolvedSegment {
    /// The struct index whose properties this segment is within.
    pub(crate) context_struct_index: i32,
    /// Position in the struct's flattened property list (0-based).
    pub(crate) property_position: usize,
    /// How this property is stored.
    pub(crate) conversion_type: ConversionType,
    /// The data type of this property.
    pub(crate) data_type: DataType,
    /// For array segments with a type filter: the struct index to match.
    pub(crate) type_filter_struct_index: Option<i32>,
    /// The struct index to descend into for the next segment.
    pub(crate) target_struct_index: Option<i32>,
}

impl ResolvedSegment {
    pub fn context_struct_index(&self) -> i32 {
        self.context_struct_index
    }

    pub fn property_position(&self) -> usize {
        self.property_position
    }

    pub fn conversion_type(&self) -> ConversionType {
        self.conversion_type
    }

    pub fn data_type(&self) -> DataType {
        self.data_type
    }

    pub fn type_filter_struct_index(&self) -> Option<i32> {
        self.type_filter_struct_index
    }

    pub fn target_struct_index(&self) -> Option<i32> {
        self.target_struct_index
    }
}

/// A validated, pre-compiled property path ready for execution.
#[derive(Debug, Clone)]
pub struct CompiledPath {
    pub(crate) segments: Vec<ResolvedSegment>,
    pub(crate) root_struct_index: i32,
    pub(crate) leaf_data_type: DataType,
}

impl CompiledPath {
    pub fn segments(&self) -> &[ResolvedSegment] {
        &self.segments
    }

    pub fn root_struct_id(&self) -> crate::types::StructId {
        crate::types::StructId(self.root_struct_index)
    }

    pub fn leaf_data_type(&self) -> DataType {
        self.leaf_data_type
    }
}

/// A compiled multi-field path: shared prefix segments + multiple leaf fields.
/// All leaf fields must live on the same struct and have the same FromDataCore type.
#[derive(Debug, Clone)]
pub struct CompiledMultiPath {
    pub(crate) prefix_segments: Vec<ResolvedSegment>,
    pub(crate) root_struct_index: i32,
    pub(crate) leaf_segments: Vec<ResolvedSegment>,
}

impl CompiledMultiPath {
    pub fn root_struct_id(&self) -> crate::types::StructId {
        crate::types::StructId(self.root_struct_index)
    }
}

/// Find a struct by name in the database, returning its index. O(1) via cached map.
fn find_struct_by_name(db: &Database, name: &str) -> Result<i32, QueryError> {
    db.struct_index_by_name(name)
        .ok_or_else(|| QueryError::StructNotFound {
            name: name.to_string(),
        })
}

/// Return the name of a struct by index.
fn struct_name<'a>(db: &'a Database, struct_index: i32) -> &'a str {
    db.resolve_string2(db.struct_def(struct_index).name_offset)
}

/// Check whether `child_si` is equal to or inherits from `parent_si`.
/// Walks the parent_type_index chain of `child_si`.
fn type_inherits_from(db: &Database, child_si: i32, parent_si: i32) -> bool {
    let mut current = child_si;
    loop {
        if current == parent_si {
            return true;
        }
        let sd = db.struct_def(current);
        if sd.parent_type_index == -1 {
            return false;
        }
        current = sd.parent_type_index;
    }
}

/// Compile a rooted path where the first segment is the type name.
///
/// Example: `"EntityClassDefinition.Components[SCItemShieldGeneratorParams].MaxShieldHealth"`
///
/// The first segment (`EntityClassDefinition`) is resolved as a struct type name.
/// The remaining segments are property paths starting from that type.
pub fn compile_rooted<'a, T: FromDataCore<'a>>(
    db: &'a Database<'a>,
    path: &str,
) -> Result<CompiledPath, QueryError> {
    let raw_segments = parse_path(path)?;
    if raw_segments.is_empty() {
        return Err(QueryError::PathParse {
            position: 0,
            message: "path is empty".into(),
        });
    }

    let root_seg = &raw_segments[0];
    if root_seg.type_filter.is_some() || root_seg.is_array {
        return Err(QueryError::PathParse {
            position: 0,
            message: format!("root type '{}' must not have brackets", root_seg.name),
        });
    }

    let root_struct_index = find_struct_by_name(db, &root_seg.name)?;

    if raw_segments.len() == 1 {
        return Err(QueryError::PathParse {
            position: 0,
            message: "rooted path needs at least one property after the type name".into(),
        });
    }

    compile_segments::<T>(db, root_struct_index, &raw_segments[1..])
}

/// Compile a property path string into a `CompiledPath`, validating schema at each step.
///
/// `struct_index` is the root struct to start from. `T` determines the expected leaf type.
pub fn compile_path<'a, T: FromDataCore<'a>>(
    db: &'a Database<'a>,
    struct_index: i32,
    path: &str,
) -> Result<CompiledPath, QueryError> {
    let raw_segments = parse_path(path)?;
    compile_segments::<T>(db, struct_index, &raw_segments)
}

fn compile_segments<'a, T: FromDataCore<'a>>(
    db: &'a Database<'a>,
    root_struct_index: i32,
    raw_segments: &[crate::query::path::PathSegment],
) -> Result<CompiledPath, QueryError> {
    let mut current_struct_index = root_struct_index;
    let mut resolved = Vec::with_capacity(raw_segments.len());

    for seg in raw_segments {
        // Find the property by name in the flattened property list of current_struct_index.
        let prop_indices = db.all_property_indices(current_struct_index);
        let mut found_position: Option<usize> = None;
        for (pos, &pi) in prop_indices.iter().enumerate() {
            let prop = &db.property_defs()[pi as usize];
            if db.resolve_string2(prop.name_offset) == seg.name {
                found_position = Some(pos);
                break;
            }
        }

        let property_position = found_position.ok_or_else(|| QueryError::PropertyNotFound {
            property: seg.name.clone(),
            struct_name: struct_name(db, current_struct_index).to_string(),
        })?;

        let pi = prop_indices[property_position];
        let prop = &db.property_defs()[pi as usize];

        let data_type = DataType::try_from(prop.data_type)
            .map_err(|_| QueryError::UnknownType(prop.data_type))?;
        let conversion_type = ConversionType::try_from(prop.conversion_type)
            .map_err(|_| QueryError::UnknownType(prop.conversion_type))?;

        // Validate: type filter / array syntax on non-pointer scalars
        let is_pointer_type = matches!(data_type, DataType::StrongPointer | DataType::WeakPointer);
        if (seg.is_array || seg.type_filter.is_some())
            && conversion_type == ConversionType::Attribute
            && !is_pointer_type
        {
            return Err(QueryError::TypeFilterOnScalar {
                property: seg.name.clone(),
            });
        }

        // Validate: polymorphic pointer arrays require a type filter
        if is_pointer_type && conversion_type != ConversionType::Attribute {
            if seg.type_filter.is_none() {
                return Err(QueryError::TypeFilterRequired {
                    property: seg.name.clone(),
                });
            }
        }

        // Resolve type filter if present
        let type_filter_struct_index = if let Some(filter_name) = &seg.type_filter {
            let filter_si = find_struct_by_name(db, filter_name)?;
            // Verify filter type is equal to or descends from property's base struct_index
            let base_si = prop.struct_index as i32;
            if !type_inherits_from(db, filter_si, base_si) {
                return Err(QueryError::TypeFilterMismatch {
                    filter: filter_name.to_string(),
                    expected: struct_name(db, base_si).to_string(),
                });
            }
            Some(filter_si)
        } else {
            None
        };

        // Determine target_struct_index for the next segment
        let target_struct_index = match data_type {
            DataType::Class => Some(prop.struct_index as i32),
            DataType::StrongPointer | DataType::WeakPointer | DataType::Reference => {
                // Use the type filter if specified, otherwise use the property's own struct_index
                type_filter_struct_index.or(Some(prop.struct_index as i32))
            }
            _ => None,
        };

        resolved.push(ResolvedSegment {
            context_struct_index: current_struct_index,
            property_position,
            conversion_type,
            data_type,
            type_filter_struct_index,
            target_struct_index,
        });

        // Advance to the target struct for the next segment
        match target_struct_index {
            Some(next_si) => current_struct_index = next_si,
            None => {
                // Leaf node — no further descent possible; break out
                break;
            }
        }
    }

    let leaf_segment = resolved.last().ok_or_else(|| QueryError::PathParse {
        position: 0,
        message: "path produced no segments".into(),
    })?;
    let leaf_data_type = leaf_segment.data_type;

    // Validate leaf type against what T expects
    let expected = T::expected_data_types();
    if !expected.contains(&leaf_data_type) {
        let leaf_prop_name = raw_segments
            .last()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        return Err(QueryError::LeafTypeMismatch {
            property: leaf_prop_name,
            expected,
            actual: leaf_data_type,
        });
    }

    Ok(CompiledPath {
        segments: resolved,
        root_struct_index,
        leaf_data_type,
    })
}

/// Compile a multi-field path: shared prefix + multiple leaf field names.
///
/// The prefix path must resolve to a struct-typed property (Class, StrongPointer, etc.)
/// so that leaf fields can be resolved on the target struct.
pub fn compile_multi_path<'a, T: FromDataCore<'a>>(
    db: &'a Database<'a>,
    struct_index: i32,
    prefix_path: &str,
    leaf_names: &[&str],
) -> Result<CompiledMultiPath, QueryError> {
    // Compile the prefix using Value as the leaf type (accepts everything).
    let prefix = compile_path::<Value>(db, struct_index, prefix_path)?;

    // The prefix must end at a struct-typed segment so we can resolve leaf fields on it.
    let last_seg = prefix
        .segments
        .last()
        .ok_or_else(|| QueryError::PathParse {
            position: 0,
            message: "prefix path produced no segments".into(),
        })?;
    let leaf_struct_index = last_seg
        .target_struct_index
        .ok_or_else(|| QueryError::PathParse {
            position: 0,
            message: "prefix path does not end at a struct type".into(),
        })?;

    // Resolve each leaf field on the target struct.
    let expected = T::expected_data_types();
    let prop_indices = db.all_property_indices(leaf_struct_index);
    let mut leaf_segments = Vec::with_capacity(leaf_names.len());

    for &leaf_name in leaf_names {
        let mut found_position: Option<usize> = None;
        for (pos, &pi) in prop_indices.iter().enumerate() {
            let prop = &db.property_defs()[pi as usize];
            if db.resolve_string2(prop.name_offset) == leaf_name {
                found_position = Some(pos);
                break;
            }
        }

        let position = found_position.ok_or_else(|| QueryError::PropertyNotFound {
            property: leaf_name.to_string(),
            struct_name: struct_name(db, leaf_struct_index).to_string(),
        })?;

        let pi = prop_indices[position];
        let prop = &db.property_defs()[pi as usize];
        let data_type = DataType::try_from(prop.data_type)
            .map_err(|_| QueryError::UnknownType(prop.data_type))?;
        let conversion_type = ConversionType::try_from(prop.conversion_type)
            .map_err(|_| QueryError::UnknownType(prop.conversion_type))?;

        if !expected.contains(&data_type) {
            return Err(QueryError::LeafTypeMismatch {
                property: leaf_name.to_string(),
                expected,
                actual: data_type,
            });
        }

        leaf_segments.push(ResolvedSegment {
            context_struct_index: leaf_struct_index,
            property_position: position,
            conversion_type,
            data_type,
            type_filter_struct_index: None,
            target_struct_index: None,
        });
    }

    Ok(CompiledMultiPath {
        prefix_segments: prefix.segments,
        root_struct_index: struct_index,
        leaf_segments,
    })
}

/// Rooted version of `compile_multi_path` — first segment of prefix_path is the type name.
///
/// Example:
/// ```text
/// compile_multi_rooted::<f32>(db,
///     "EntityClassDefinition.Components[SAmmoContainerComponentParams].ammoParamsRecord.projectileParams[BulletProjectileParams].damage[DamageInfo]",
///     &["DamagePhysical", "DamageEnergy", "DamageDistortion"]
/// )
/// ```
pub fn compile_multi_rooted<'a, T: FromDataCore<'a>>(
    db: &'a Database<'a>,
    prefix_path: &str,
    leaf_names: &[&str],
) -> Result<CompiledMultiPath, QueryError> {
    let raw_segments = parse_path(prefix_path)?;
    if raw_segments.is_empty() {
        return Err(QueryError::PathParse {
            position: 0,
            message: "path is empty".into(),
        });
    }

    let root_seg = &raw_segments[0];
    if root_seg.type_filter.is_some() || root_seg.is_array {
        return Err(QueryError::PathParse {
            position: 0,
            message: format!("root type '{}' must not have brackets", root_seg.name),
        });
    }

    let root_struct_index = find_struct_by_name(db, &root_seg.name)?;

    // Split the original path string on the first dot to get the property portion.
    let first_dot = prefix_path.find('.').ok_or_else(|| QueryError::PathParse {
        position: 0,
        message: "rooted multi-path needs at least one property after the type name".into(),
    })?;
    let property_path = &prefix_path[first_dot + 1..];

    compile_multi_path::<T>(db, root_struct_index, property_path, leaf_names)
}
