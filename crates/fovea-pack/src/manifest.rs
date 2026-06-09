use std::collections::BTreeMap;

use image::ImageFormat as ImageCrateFormat;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImageFormat {
    Webp,
    Jpeg,
    Png,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Webp => "webp",
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }

    pub fn image_crate_format(self) -> ImageCrateFormat {
        match self {
            Self::Webp => ImageCrateFormat::WebP,
            Self::Jpeg => ImageCrateFormat::Jpeg,
            Self::Png => ImageCrateFormat::Png,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub fn tile_count(self, tile_size: u32) -> (u32, u32) {
        (
            self.width.div_ceil(tile_size),
            self.height.div_ceil(tile_size),
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Bounds {
    pub x: Option<i64>,
    pub y: Option<i64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Metadata {
    pub vendor: Option<String>,
    pub mpp_x: Option<f64>,
    pub mpp_y: Option<f64>,
    pub objective_power: Option<f64>,
    pub background_color: Option<String>,
    pub bounds: Bounds,
    pub raw_properties: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LevelManifest {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub downsample: f64,
    pub tile_cols: u32,
    pub tile_rows: u32,
    pub tile_count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TileManifest {
    pub level: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub path: String,
    pub byte_size: u64,
    pub skipped: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AssociatedImageManifest {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub schema: String,
    pub version: String,
    pub tile_size: u32,
    pub image_format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub levels: Vec<LevelManifest>,
    pub tiles: Vec<TileManifest>,
    pub associated_images: Vec<AssociatedImageManifest>,
    pub metadata: Metadata,
    pub coordinate_consistency_max_error_px: f64,
}

#[cfg(test)]
mod tests {
    use super::{ImageFormat, Size};

    #[test]
    fn tile_count_uses_partial_edge_tiles() {
        assert_eq!(
            Size {
                width: 1025,
                height: 513
            }
            .tile_count(512),
            (3, 2)
        );
    }

    #[test]
    fn image_format_paths_are_stable() {
        assert_eq!(ImageFormat::Webp.extension(), "webp");
        assert_eq!(ImageFormat::Jpeg.extension(), "jpg");
        assert_eq!(ImageFormat::Png.extension(), "png");
    }
}
