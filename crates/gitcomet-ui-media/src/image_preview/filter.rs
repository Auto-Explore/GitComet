//! Sampling policy and kernels. Coordinates are always original/device pixels.
use fast_image_resize::{Filter, FilterType, ResizeAlg};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum ImageSamplingMode {
    #[default]
    Auto,
    Smooth,
    Pixels,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub enum ImageSmoothFilter {
    #[default]
    Spline36,
    Robidoux,
    Hann,
}

macro_rules! names {
    ($ty:ty, $( $variant:ident => ($key:literal, $label:literal) ),+ $(,)?) => {
        impl $ty {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
            pub fn key(self) -> &'static str { match self { $(Self::$variant => $key),+ } }
            pub fn label(self) -> &'static str { match self { $(Self::$variant => $label),+ } }
            pub fn parse(value: Option<&str>) -> Self {
                match value { $(Some($key) => Self::$variant),+, _ => Self::default() }
            }
        }
    };
}
names!(ImageSamplingMode, Auto => ("auto", "Auto"), Smooth => ("smooth", "Smooth"), Pixels => ("pixels", "Pixels"));
names!(ImageSmoothFilter, Spline36 => ("spline36", "Spline36"), Robidoux => ("robidoux", "Robidoux"), Hann => ("hann", "Hann"));

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(super) enum Sampling {
    Nearest,
    Smooth(ImageSmoothFilter),
}

pub(super) fn sampling(mode: ImageSamplingMode, filter: ImageSmoothFilter, scale: f64) -> Sampling {
    if mode == ImageSamplingMode::Pixels || (mode == ImageSamplingMode::Auto && scale >= 3.0) {
        Sampling::Nearest
    } else {
        Sampling::Smooth(filter)
    }
}

impl Sampling {
    pub(super) fn weight(self, x: f64) -> f64 {
        match self {
            Self::Nearest => {
                if x.abs() < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Smooth(ImageSmoothFilter::Spline36) => spline36(x),
            Self::Smooth(ImageSmoothFilter::Robidoux) => robidoux(x),
            Self::Smooth(ImageSmoothFilter::Hann) => hann(x),
        }
    }
    pub(super) fn algorithm(self) -> ResizeAlg {
        match self {
            Self::Nearest => ResizeAlg::Nearest,
            Self::Smooth(filter) => {
                let (kernel, support): (fn(f64) -> f64, f64) = match filter {
                    ImageSmoothFilter::Spline36 => (spline36, 3.0),
                    ImageSmoothFilter::Robidoux => (robidoux, 2.0),
                    ImageSmoothFilter::Hann => (hann, 3.0),
                };
                ResizeAlg::Convolution(FilterType::Custom(
                    Filter::new(filter.key(), kernel, support).expect("positive filter support"),
                ))
            }
        }
    }
    pub(super) fn support(self) -> f64 {
        match self {
            Self::Nearest => 0.0,
            Self::Smooth(ImageSmoothFilter::Robidoux) => 2.0,
            Self::Smooth(_) => 3.0,
        }
    }
}

// zimg/Avisynth's interpolating Spline36, not a cubic B-spline.
// https://github.com/sekrit-twc/zimg/blob/master/src/zimg/resize/filter.cpp
fn spline36(x: f64) -> f64 {
    let x = x.abs();
    if x < 1.0 {
        ((13.0 / 11.0 * x - 453.0 / 209.0) * x - 3.0 / 209.0) * x + 1.0
    } else if x < 2.0 {
        let x = x - 1.0;
        ((-6.0 / 11.0 * x + 270.0 / 209.0) * x - 156.0 / 209.0) * x
    } else if x < 3.0 {
        let x = x - 2.0;
        ((1.0 / 11.0 * x - 45.0 / 209.0) * x + 26.0 / 209.0) * x
    } else {
        0.0
    }
}

fn robidoux(x: f64) -> f64 {
    let x = x.abs();
    const B: f64 = 0.378_215_755_093_998_7;
    const C: f64 = (1.0 - B) / 2.0;
    if x < 1.0 {
        ((12.0 - 9.0 * B - 6.0 * C) * x.powi(3) + (-18.0 + 12.0 * B + 6.0 * C) * x * x + 6.0
            - 2.0 * B)
            / 6.0
    } else if x < 2.0 {
        ((-B - 6.0 * C) * x.powi(3)
            + (6.0 * B + 30.0 * C) * x * x
            + (-12.0 * B - 48.0 * C) * x
            + 8.0 * B
            + 24.0 * C)
            / 6.0
    } else {
        0.0
    }
}

fn hann(x: f64) -> f64 {
    let x = x.abs();
    if x == 0.0 {
        return 1.0;
    }
    if x >= 3.0 {
        return 0.0;
    }
    let p = std::f64::consts::PI * x;
    p.sin() / p * (1.0 + (p / 3.0).cos()) * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_sampling_threshold_and_overrides() {
        for f in ImageSmoothFilter::ALL {
            assert_eq!(
                sampling(ImageSamplingMode::Auto, *f, 2.99),
                Sampling::Smooth(*f)
            );
            for scale in [3.0, 3.01, 64.0] {
                assert_eq!(
                    sampling(ImageSamplingMode::Auto, *f, scale),
                    Sampling::Nearest
                );
                assert_eq!(
                    sampling(ImageSamplingMode::Smooth, *f, scale),
                    Sampling::Smooth(*f)
                );
            }
            assert_eq!(
                sampling(ImageSamplingMode::Pixels, *f, 0.1),
                Sampling::Nearest
            );
        }
    }
    #[test]
    fn kernel_support_symmetry_and_interpolation() {
        for kernel in [spline36, robidoux, hann] {
            for x in [0.25, 0.75, 1.5, 2.5, 3.0, 4.0] {
                assert_eq!(kernel(x), kernel(-x));
            }
            assert_eq!(kernel(3.0), 0.0);
        }
        for kernel in [spline36, hann] {
            assert_eq!(kernel(0.0), 1.0);
            for x in [1.0, 2.0, 3.0] {
                assert!(kernel(x).abs() < 1e-12);
            }
        }
    }
}
