//! Text formatting helpers for the classic DataCore XML exporter.
//!
//! This module owns legacy scalar spelling, XML escaping, element-name labels,
//! and minimal XmlConvert-style root-name encoding used by the classic writer.

use std::io::Write;

use crate::enums::DataType;
use crate::error::ExportError;

pub(super) fn data_type_element_name(data_type: DataType) -> &'static str {
    match data_type {
        DataType::Boolean => "Boolean",
        DataType::SByte => "SByte",
        DataType::Int16 => "Int16",
        DataType::Int32 => "Int32",
        DataType::Int64 => "Int64",
        DataType::Byte => "Byte",
        DataType::UInt16 => "UInt16",
        DataType::UInt32 => "UInt32",
        DataType::UInt64 => "UInt64",
        DataType::String => "String",
        DataType::Single => "Single",
        DataType::Double => "Double",
        DataType::Locale => "Locale",
        DataType::Guid => "Guid",
        DataType::EnumChoice => "EnumChoice",
        DataType::Class => "Class",
        DataType::StrongPointer => "StrongPointer",
        DataType::WeakPointer => "WeakPointer",
        DataType::Reference => "Reference",
    }
}

pub(super) fn format_bool(value: bool) -> &'static str {
    if value {
        "True"
    } else {
        "False"
    }
}

pub(super) fn format_single(value: f32) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value == f32::INFINITY {
        return "Infinity".to_string();
    }
    if value == f32::NEG_INFINITY {
        return "-Infinity".to_string();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_string()
        } else {
            "0".to_string()
        };
    }

    let abs = value.abs();
    if !(1e-4..1e9).contains(&abs) {
        return format_scientific(value as f64, 7);
    }

    value.to_string()
}

pub(super) fn format_double(value: f64) -> String {
    format_general(value, 15, 15)
}

fn format_general(value: f64, precision: usize, scientific_exponent: i32) -> String {
    if value.is_nan() {
        return "NaN".to_string();
    }
    if value == f64::INFINITY {
        return "Infinity".to_string();
    }
    if value == f64::NEG_INFINITY {
        return "-Infinity".to_string();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0".to_string()
        } else {
            "0".to_string()
        };
    }

    let exponent = rounded_scientific_exponent(value, precision);
    if exponent < -4 || exponent >= scientific_exponent {
        return format_scientific(value, precision);
    }

    format_fixed(value, precision, exponent)
}

fn format_scientific(value: f64, precision: usize) -> String {
    let decimals = precision.saturating_sub(1);
    let raw = format!("{value:.decimals$e}");
    let (mantissa, exponent) = raw
        .split_once('e')
        .or_else(|| raw.split_once('E'))
        .unwrap_or((raw.as_str(), "0"));
    let mantissa = trim_fixed(mantissa.to_string());
    let exponent = exponent.parse::<i32>().unwrap_or(0);
    let sign = if exponent < 0 { '-' } else { '+' };
    format!("{mantissa}E{sign}{:02}", exponent.abs())
}

fn rounded_scientific_exponent(value: f64, precision: usize) -> i32 {
    let decimals = precision.saturating_sub(1);
    let raw = format!("{value:.decimals$e}");
    raw.split_once('e')
        .or_else(|| raw.split_once('E'))
        .and_then(|(_, exponent)| exponent.parse::<i32>().ok())
        .unwrap_or(0)
}

fn format_fixed(value: f64, precision: usize, exponent: i32) -> String {
    let decimals = precision as i32 - exponent - 1;
    if decimals >= 0 {
        return trim_fixed(format!("{value:.decimals$}", decimals = decimals as usize));
    }

    let scale = 10f64.powi(-decimals);
    let rounded = (value / scale).round() * scale;
    trim_fixed(format!("{rounded:.0}"))
}

fn trim_fixed(mut value: String) -> String {
    if let Some(dot) = value.find('.') {
        while value.ends_with('0') {
            value.pop();
        }
        if value.len() == dot + 1 {
            value.pop();
        }
    }
    value
}

pub(super) fn encode_xml_name(name: &str) -> String {
    let mut out = String::new();
    for (i, ch) in name.chars().enumerate() {
        if is_valid_xml_name_char(ch, i == 0) {
            out.push(ch);
        } else {
            out.push_str(&format!("_x{:04X}_", ch as u32));
        }
    }
    out
}

fn is_valid_xml_name_char(ch: char, first: bool) -> bool {
    if first {
        ch == '_' || ch == ':' || ch.is_ascii_alphabetic()
    } else {
        ch == '_' || ch == ':' || ch == '-' || ch == '.' || ch.is_ascii_alphanumeric()
    }
}

pub(super) fn write_escaped_text(w: &mut impl Write, value: &str) -> Result<(), ExportError> {
    write_escaped(w, value, false)
}

pub(super) fn write_escaped_attr(w: &mut impl Write, value: &str) -> Result<(), ExportError> {
    write_escaped(w, value, true)
}

fn write_escaped(w: &mut impl Write, value: &str, attr: bool) -> Result<(), ExportError> {
    let bytes = value.as_bytes();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        let escaped = match *byte {
            b'&' => Some(b"&amp;".as_slice()),
            b'<' => Some(b"&lt;".as_slice()),
            b'>' => Some(b"&gt;".as_slice()),
            b'"' if attr => Some(b"&quot;".as_slice()),
            _ => None,
        };
        if let Some(escaped) = escaped {
            if start < index {
                w.write_all(&bytes[start..index])?;
            }
            w.write_all(escaped)?;
            start = index + 1;
        }
    }
    if start < bytes.len() {
        w.write_all(&bytes[start..])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{encode_xml_name, format_bool, format_double, format_single};

    #[test]
    fn booleans_match_legacy_csharp_casing() {
        assert_eq!(format_bool(true), "True");
        assert_eq!(format_bool(false), "False");
    }

    #[test]
    fn single_uses_legacy_csharp_general_format() {
        assert_eq!(format_single(1e9), "1E+09");
        assert_eq!(format_single(1.739092e36), "1.739092E+36");
        assert_eq!(format_single(0.0000536193), "5.36193E-05");
        assert_eq!(format_single(0.00009), "9E-05");
        assert_eq!(format_single(0.00000125), "1.25E-06");
        assert_eq!(format_single(0.0001), "0.0001");
        assert_eq!(format_single(10000000.0), "10000000");
        assert_eq!(format_single(100000000.0), "100000000");
        assert_eq!(format_single(295661000.0), "295661000");
        assert_eq!(format_single(-0.0), "-0");
        assert_eq!(format_single(-2.5089607), "-2.5089607");
        assert_eq!(format_single(123456.7), "123456.7");
    }

    #[test]
    fn double_uses_legacy_csharp_general_format() {
        assert_eq!(format_double(1e15), "1E+15");
        assert_eq!(format_double(0.00001), "1E-05");
        assert_eq!(format_double(12345678901234.5), "12345678901234.5");
    }

    #[test]
    fn invalid_xml_name_characters_are_encoded() {
        assert_eq!(encode_xml_name("1 Bad"), "_x0031__x0020_Bad");
        assert_eq!(encode_xml_name("Mission.Init"), "Mission.Init");
    }
}
