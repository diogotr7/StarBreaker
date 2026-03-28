use starbreaker_datacore::query::path::parse_path;

#[test]
fn parse_simple_field() {
    let segments = parse_path("name").unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(segments[0].name, "name");
    assert_eq!(segments[0].type_filter, None);
    assert!(!segments[0].is_array);
}

#[test]
fn parse_dotted_path() {
    let segments = parse_path("Geometry.Geometry.Geometry.path").unwrap();
    assert_eq!(segments.len(), 4);
    assert_eq!(segments[0].name, "Geometry");
    assert_eq!(segments[3].name, "path");
    assert!(segments.iter().all(|s| s.type_filter.is_none()));
    assert!(segments.iter().all(|s| !s.is_array));
}

#[test]
fn parse_typed_array_filter() {
    let segments = parse_path("Components[SGeometryResourceParams].Geometry").unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].name, "Components");
    assert_eq!(
        segments[0].type_filter.as_deref(),
        Some("SGeometryResourceParams")
    );
    assert!(segments[0].is_array);
    assert_eq!(segments[1].name, "Geometry");
    assert!(!segments[1].is_array);
}

#[test]
fn parse_unfiltered_array() {
    let segments = parse_path("items[].name").unwrap();
    assert_eq!(segments.len(), 2);
    assert_eq!(segments[0].name, "items");
    assert!(segments[0].is_array);
    assert_eq!(segments[0].type_filter, None);
}

#[test]
fn parse_empty_path_fails() {
    assert!(parse_path("").is_err());
}

#[test]
fn parse_trailing_dot_fails() {
    assert!(parse_path("foo.").is_err());
}

#[test]
fn parse_leading_dot_fails() {
    assert!(parse_path(".foo").is_err());
}

#[test]
fn parse_unclosed_bracket_fails() {
    assert!(parse_path("foo[bar").is_err());
}

#[test]
fn parse_bracket_without_name_fails() {
    assert!(parse_path("[Foo].bar").is_err());
}

#[test]
fn parse_double_dot_fails() {
    assert!(parse_path("foo..bar").is_err());
}

#[test]
fn parse_full_entity_path() {
    let segments =
        parse_path("Components[SGeometryResourceParams].Geometry.Geometry.Geometry.path").unwrap();
    assert_eq!(segments.len(), 5);
    assert_eq!(segments[0].name, "Components");
    assert_eq!(
        segments[0].type_filter.as_deref(),
        Some("SGeometryResourceParams")
    );
    assert!(segments[0].is_array);
    assert_eq!(segments[1].name, "Geometry");
    assert_eq!(segments[4].name, "path");
}
