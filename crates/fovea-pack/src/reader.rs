use std::{collections::BTreeMap, path::Path};

use anyhow::{anyhow, Context, Result};
use image::RgbaImage;
use openslide_rs::{Address, OpenSlide, Region, Size as OpenSlideSize};

use crate::manifest::{Bounds, Metadata, Size};

pub trait SlideReader: Send + Sync {
    fn dimensions(&self) -> Result<Size>;
    fn level_count(&self) -> Result<usize>;
    fn level_dimensions(&self, level: usize) -> Result<Size>;
    fn level_downsample(&self, level: usize) -> Result<f64>;
    fn properties(&self) -> SlideProperties;
    fn associated_image_names(&self) -> Result<Vec<String>>;
    fn read_associated_image_rgba(&self, name: &str) -> Result<(Size, RgbaImage)>;
    fn read_region_rgba(
        &self,
        level: usize,
        x: i64,
        y: i64,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage>;
}

#[derive(Clone, Debug)]
pub struct SlideProperties {
    pub raw: BTreeMap<String, String>,
}

impl SlideProperties {
    pub fn metadata(&self) -> Metadata {
        Metadata {
            vendor: self.get_string("openslide.vendor"),
            mpp_x: self.get_f64("openslide.mpp-x"),
            mpp_y: self.get_f64("openslide.mpp-y"),
            objective_power: self
                .get_f64("openslide.objective-power")
                .or_else(|| self.get_f64("aperio.AppMag")),
            background_color: self.get_string("openslide.background-color"),
            bounds: Bounds {
                x: self.get_i64("openslide.bounds-x"),
                y: self.get_i64("openslide.bounds-y"),
                width: self.get_u32("openslide.bounds-width"),
                height: self.get_u32("openslide.bounds-height"),
            },
            raw_properties: self.raw.clone(),
        }
    }

    pub fn background_rgb(&self) -> [u8; 3] {
        self.get_string("openslide.background-color")
            .and_then(|value| parse_hex_rgb(&value))
            .unwrap_or([255, 255, 255])
    }

    fn get_string(&self, name: &str) -> Option<String> {
        self.raw
            .get(name)
            .cloned()
            .filter(|value| !value.is_empty())
    }

    fn get_f64(&self, name: &str) -> Option<f64> {
        self.raw.get(name).and_then(|value| value.parse().ok())
    }

    fn get_i64(&self, name: &str) -> Option<i64> {
        self.raw.get(name).and_then(|value| value.parse().ok())
    }

    fn get_u32(&self, name: &str) -> Option<u32> {
        self.raw.get(name).and_then(|value| value.parse().ok())
    }
}

pub struct OpenSlideReader {
    inner: OpenSlide,
}

impl OpenSlideReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            inner: OpenSlide::new(path.as_ref()).with_context(|| {
                format!(
                    "failed to open WSI with OpenSlide: {}",
                    path.as_ref().display()
                )
            })?,
        })
    }
}

impl SlideReader for OpenSlideReader {
    fn dimensions(&self) -> Result<Size> {
        self.level_dimensions(0)
    }

    fn level_count(&self) -> Result<usize> {
        Ok(self.inner.get_level_count()? as usize)
    }

    fn level_dimensions(&self, level: usize) -> Result<Size> {
        let size = self.inner.get_level_dimensions(level as u32)?;
        Ok(to_size(size))
    }

    fn level_downsample(&self, level: usize) -> Result<f64> {
        Ok(self.inner.get_level_downsample(level as u32)?)
    }

    fn properties(&self) -> SlideProperties {
        let raw = self
            .inner
            .get_property_names()
            .into_iter()
            .filter_map(|name| {
                self.inner
                    .get_property_value(&name)
                    .ok()
                    .map(|value| (name, value))
            })
            .collect();

        SlideProperties { raw }
    }

    fn associated_image_names(&self) -> Result<Vec<String>> {
        Ok(self.inner.get_associated_image_names()?)
    }

    fn read_associated_image_rgba(&self, name: &str) -> Result<(Size, RgbaImage)> {
        let image = self.inner.read_associated_image_rgba(name)?;
        let size = Size {
            width: image.width(),
            height: image.height(),
        };

        Ok((size, image))
    }

    fn read_region_rgba(
        &self,
        level: usize,
        x: i64,
        y: i64,
        width: u32,
        height: u32,
    ) -> Result<RgbaImage> {
        if x < 0 || y < 0 {
            return Err(anyhow!(
                "negative OpenSlide region coordinates are unsupported"
            ));
        }

        Ok(self.inner.read_image_rgba(&Region {
            size: OpenSlideSize {
                w: width,
                h: height,
            },
            level: level as u32,
            address: Address {
                x: x.try_into()?,
                y: y.try_into()?,
            },
        })?)
    }
}

fn to_size(size: OpenSlideSize) -> Size {
    Size {
        width: size.w,
        height: size.h,
    }
}

fn parse_hex_rgb(value: &str) -> Option<[u8; 3]> {
    let value = value.trim().trim_start_matches('#');

    if value.len() != 6 {
        return None;
    }

    Some([
        u8::from_str_radix(&value[0..2], 16).ok()?,
        u8::from_str_radix(&value[2..4], 16).ok()?,
        u8::from_str_radix(&value[4..6], 16).ok()?,
    ])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::SlideProperties;

    #[test]
    fn extracts_common_metadata() {
        let raw = BTreeMap::from([
            ("openslide.vendor".to_string(), "aperio".to_string()),
            ("openslide.mpp-x".to_string(), "0.25".to_string()),
            ("openslide.mpp-y".to_string(), "0.26".to_string()),
            ("aperio.AppMag".to_string(), "40".to_string()),
            ("openslide.bounds-x".to_string(), "10".to_string()),
            ("openslide.bounds-width".to_string(), "2048".to_string()),
        ]);
        let metadata = SlideProperties { raw }.metadata();

        assert_eq!(metadata.vendor.as_deref(), Some("aperio"));
        assert_eq!(metadata.mpp_x, Some(0.25));
        assert_eq!(metadata.mpp_y, Some(0.26));
        assert_eq!(metadata.objective_power, Some(40.0));
        assert_eq!(metadata.bounds.x, Some(10));
        assert_eq!(metadata.bounds.width, Some(2048));
    }
}
