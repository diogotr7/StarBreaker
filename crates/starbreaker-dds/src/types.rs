use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

/// DDS pixel format descriptor (32 bytes).
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
pub struct DdsPixelFormat {
    /// Always 32.
    pub size: u32,
    pub flags: u32,
    /// e.g. b"DX10", b"DXT1", b"DXT5", b"BC4U", b"BC5U"
    pub four_cc: [u8; 4],
    pub rgb_bit_count: u32,
    pub r_bit_mask: u32,
    pub g_bit_mask: u32,
    pub b_bit_mask: u32,
    pub a_bit_mask: u32,
}

const _: () = assert!(size_of::<DdsPixelFormat>() == 32);

/// DDS main header (124 bytes). Follows the 4-byte "DDS " magic in the file.
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
pub struct DdsHeader {
    /// Always 124.
    pub size: u32,
    pub flags: u32,
    pub height: u32,
    pub width: u32,
    pub pitch_or_linear_size: u32,
    pub depth: u32,
    pub mipmap_count: u32,
    /// CryEngine stores custom data here (alpha bit depth, brightness, colors, etc.)
    pub reserved1: [u32; 11],
    pub pixel_format: DdsPixelFormat,
    pub surface_flags: u32,
    pub cubemap_flags: u32,
    pub reserved2: [u32; 3],
}

const _: () = assert!(size_of::<DdsHeader>() == 124);

/// DX10 extended header (20 bytes). Present when pixel_format.four_cc == b"DX10".
#[derive(Debug, Clone, Copy, FromBytes, IntoBytes, KnownLayout, Immutable)]
#[repr(C, packed)]
pub struct DdsHeaderDxt10 {
    pub dxgi_format: u32,
    pub resource_dimension: u32,
    pub misc_flag: u32,
    pub array_size: u32,
    pub misc_flags2: u32,
}

const _: () = assert!(size_of::<DdsHeaderDxt10>() == 20);

/// Block-compressed DXGI formats we support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxgiFormat {
    BC1Unorm,
    BC1UnormSrgb,
    BC3Unorm,
    BC3UnormSrgb,
    BC4Unorm,
    BC4Snorm,
    BC5Unorm,
    BC5Snorm,
    BC6hUf16,
    BC7Unorm,
    BC7UnormSrgb,
}

impl DxgiFormat {
    /// Try to parse from the DXGI format integer in a DX10 header.
    pub fn from_dxgi(value: u32) -> Option<Self> {
        match value {
            71 => Some(Self::BC1Unorm),
            72 => Some(Self::BC1UnormSrgb),
            77 => Some(Self::BC3Unorm),
            78 => Some(Self::BC3UnormSrgb),
            80 => Some(Self::BC4Unorm),
            81 => Some(Self::BC4Snorm),
            83 => Some(Self::BC5Unorm),
            84 => Some(Self::BC5Snorm),
            95 => Some(Self::BC6hUf16),
            98 => Some(Self::BC7Unorm),
            99 => Some(Self::BC7UnormSrgb),
            _ => None,
        }
    }

    /// Try to parse from FourCC bytes in the pixel format header.
    pub fn from_four_cc(four_cc: &[u8; 4]) -> Option<Self> {
        match four_cc {
            b"DXT1" => Some(Self::BC1Unorm),
            b"DXT3" => Some(Self::BC3Unorm), // BC2 not supported; approximate as BC3
            b"DXT5" => Some(Self::BC3Unorm),
            b"ATI1" => Some(Self::BC4Unorm),
            b"ATI2" => Some(Self::BC5Unorm),
            b"BC4U" => Some(Self::BC4Unorm),
            b"BC4S" => Some(Self::BC4Snorm),
            b"BC5U" => Some(Self::BC5Unorm),
            b"BC5S" => Some(Self::BC5Snorm),
            _ => None,
        }
    }

    /// Block size in bytes: 8 for BC1/BC4, 16 for BC3/BC5/BC6H/BC7.
    pub fn block_size(self) -> usize {
        match self {
            Self::BC1Unorm | Self::BC1UnormSrgb | Self::BC4Unorm | Self::BC4Snorm => 8,
            Self::BC3Unorm
            | Self::BC3UnormSrgb
            | Self::BC5Unorm
            | Self::BC5Snorm
            | Self::BC6hUf16
            | Self::BC7Unorm
            | Self::BC7UnormSrgb => 16,
        }
    }
}

/// Magic bytes at the start of every DDS file.
pub const DDS_MAGIC: [u8; 4] = *b"DDS ";
