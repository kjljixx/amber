//! Color families and the color pipeline shared by every element.
//!
//! A palette role names a family (resolved through the scene's lightness,
//! saturation and contrast) or an exact hex color. Elements ask for a role plus
//! a `shade` (lighter or darker); the pipeline adds per-point variation,
//! then optionally pulls the color toward the scene's haze by `depth`.
use crate::color::{Oklab, Rgb, rgb_to_oklab};

// OKLCH families; local conversion keeps the model away from raw pixel math.
pub(crate) const COLORS: &[(&str, f32, f32, f32)] = &[
    ("red", 25.0, 0.20, 0.62),
    ("coral", 35.0, 0.15, 0.72),
    ("orange", 55.0, 0.18, 0.70),
    ("amber", 75.0, 0.15, 0.76),
    ("yellow", 100.0, 0.17, 0.90),
    ("lime", 125.0, 0.17, 0.84),
    ("green", 150.0, 0.14, 0.65),
    ("teal", 185.0, 0.11, 0.65),
    ("cyan", 205.0, 0.13, 0.80),
    ("sky", 235.0, 0.10, 0.80),
    ("blue", 260.0, 0.18, 0.58),
    ("indigo", 275.0, 0.15, 0.38),
    ("violet", 300.0, 0.16, 0.62),
    ("magenta", 335.0, 0.22, 0.65),
    ("pink", 355.0, 0.12, 0.80),
    ("brown", 55.0, 0.07, 0.42),
    ("cream", 85.0, 0.04, 0.92),
    ("white", 250.0, 0.01, 0.97),
    ("grey", 250.0, 0.015, 0.62),
    ("black", 270.0, 0.015, 0.14),
];

pub(crate) fn family(name: &str) -> Option<(f32, f32, f32)> {
    COLORS
        .iter()
        .find(|(id, _, _, _)| *id == name)
        .map(|(_, hue, chroma, light)| (*hue, *chroma, *light))
}

/// A palette entry: a named color family, or an exact `#rrggbb` color.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(try_from = "String", into = "String")]
pub enum ColorSpec {
    Family(String),
    Rgb(Rgb),
}

impl TryFrom<String> for ColorSpec {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        if let Some(hex) = text.strip_prefix('#') {
            let valid = hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit());
            if !valid {
                return Err(format!("'{text}' is not a #rrggbb color"));
            }
            let channel = |range: std::ops::Range<usize>| u8::from_str_radix(&hex[range], 16);
            return match (channel(0..2), channel(2..4), channel(4..6)) {
                (Ok(r), Ok(g), Ok(b)) => Ok(Self::Rgb([r, g, b])),
                _ => Err(format!("'{text}' is not a #rrggbb color")),
            };
        }
        if family(&text).is_some() {
            Ok(Self::Family(text))
        } else {
            let names: Vec<_> = COLORS.iter().map(|(name, ..)| *name).collect();
            Err(format!(
                "unknown color family '{text}'; use #rrggbb or one of: {}",
                names.join(", ")
            ))
        }
    }
}

impl From<ColorSpec> for String {
    fn from(spec: ColorSpec) -> String {
        match spec {
            ColorSpec::Family(name) => name,
            ColorSpec::Rgb([r, g, b]) => format!("#{r:02x}{g:02x}{b:02x}"),
        }
    }
}

/// Lightness, chroma and hue (degrees) before conversion to Oklab.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Lch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

impl Lch {
    pub fn to_oklab(self) -> Oklab {
        let angle = self.h.to_radians();
        [self.l, self.c * angle.cos(), self.c * angle.sin()]
    }
}

/// A scene's global lightness, contrast and saturation, as multipliers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Tone {
    base: f32,
    contrast: f32,
    chroma: f32,
}

impl Tone {
    pub fn new(lightness: f32, saturation: f32, contrast: f32) -> Self {
        Self {
            base: 0.28 + lightness * 0.15,
            contrast: 0.10 + contrast / 3.0 * 0.24,
            chroma: 0.5 + 0.7 * saturation / 4.0,
        }
    }
}

impl ColorSpec {
    /// The color before random variation. `shade` runs from about -1 (dark) to
    /// 1 (light) and is scaled by the scene's contrast.
    pub(crate) fn lch(&self, tone: &Tone, shade: f32) -> Lch {
        match self {
            Self::Family(name) => {
                let (hue, chroma, preferred) =
                    family(name).expect("color families are checked when a scene is validated");
                Lch {
                    l: (tone.base + shade * tone.contrast) * 0.6 + preferred * 0.4,
                    c: chroma * tone.chroma,
                    h: hue,
                }
            }
            // Exact colors keep their own chroma; only shade moves them.
            Self::Rgb(rgb) => {
                let [l, a, b] = rgb_to_oklab(*rgb);
                Lch {
                    l: l + shade * tone.contrast * 0.6,
                    c: a.hypot(b),
                    h: b.atan2(a).to_degrees(),
                }
            }
        }
    }
}
