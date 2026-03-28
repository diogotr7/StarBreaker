use crate::error::QueryError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathSegment {
    pub name: String,
    pub type_filter: Option<String>,
    pub is_array: bool,
}

pub fn parse_path(path: &str) -> Result<Vec<PathSegment>, QueryError> {
    if path.is_empty() {
        return Err(QueryError::PathParse {
            position: 0,
            message: "path is empty".into(),
        });
    }

    let mut segments = Vec::new();
    let mut pos = 0;

    for part in path.split('.') {
        if part.is_empty() {
            return Err(QueryError::PathParse {
                position: pos,
                message: "empty segment (double dot or leading/trailing dot)".into(),
            });
        }

        let segment = parse_segment(part, pos)?;
        pos += part.len() + 1;
        segments.push(segment);
    }

    Ok(segments)
}

fn parse_segment(s: &str, base_pos: usize) -> Result<PathSegment, QueryError> {
    if let Some(bracket_start) = s.find('[') {
        let name = &s[..bracket_start];
        if name.is_empty() {
            return Err(QueryError::PathParse {
                position: base_pos,
                message: "segment has no name before '['".into(),
            });
        }

        if !s.ends_with(']') {
            return Err(QueryError::PathParse {
                position: base_pos + bracket_start,
                message: "unclosed bracket".into(),
            });
        }

        let filter_content = &s[bracket_start + 1..s.len() - 1];
        let type_filter = if filter_content.is_empty() {
            None
        } else {
            Some(filter_content.to_string())
        };

        Ok(PathSegment {
            name: name.to_string(),
            type_filter,
            is_array: true,
        })
    } else {
        Ok(PathSegment {
            name: s.to_string(),
            type_filter: None,
            is_array: false,
        })
    }
}
