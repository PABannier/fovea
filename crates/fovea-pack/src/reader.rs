use std::{collections::BTreeMap, path::Path};

use anyhow::{anyhow, Context, Result};
use image::RgbaImage;
use openslide_rs::{Address, OpenSlide, Region, Size as OpenSlideSize};

use crate::manifest::Size;

#[derive(Clone, Debug)]
pub struct SlideProperties {
    pub raw: BTreeMap<String, String>,
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

    pub fn level_count(&self) -> Result<usize> {
        Ok(self.inner.get_level_count()? as usize)
    }

    pub fn level_dimensions(&self, level: usize) -> Result<Size> {
        let size = self.inner.get_level_dimensions(level as u32)?;
        Ok(to_size(size))
    }

    pub fn level_downsample(&self, level: usize) -> Result<f64> {
        Ok(self.inner.get_level_downsample(level as u32)?)
    }

    pub fn properties(&self) -> SlideProperties {
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

    pub fn read_region_rgba(
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
