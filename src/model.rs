use serde::{Deserialize, Serialize};

/// One English Metric Unit. Document geometry never passes through pixels or floats.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct Emu(pub i64);

impl Emu {
    pub const PER_INCH: Self = Self(914_400);

    pub const fn inches(value: i64) -> Self {
        Self(value * Self::PER_INCH.0)
    }
}

/// OOXML angles use 60,000 units per degree.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Angle60000(pub i32);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeColor {
    Dark1,
    Light1,
    Dark2,
    Light2,
    Accent1,
    Accent2,
    Accent3,
    Accent4,
    Accent5,
    Accent6,
    Hyperlink,
    FollowedHyperlink,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum ColorRef {
    Theme(ThemeColor),
    ExplicitSrgb(u32),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeFont {
    Major,
    Minor,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum FontRef {
    Theme(ThemeFont),
    Explicit(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SlideSize {
    pub width: Emu,
    pub height: Emu,
}

impl SlideSize {
    pub const WIDESCREEN: Self = Self {
        width: Emu(12_192_000),
        height: Emu(6_858_000),
    };

    pub const CLASSIC_4_3: Self = Self {
        width: Emu(9_144_000),
        height: Emu(6_858_000),
    };

    pub const fn is_valid(self) -> bool {
        self.width.0 > 0 && self.height.0 > 0
    }
}

#[cfg(test)]
mod tests {
    use super::{Angle60000, ColorRef, Emu, FontRef, SlideSize, ThemeColor, ThemeFont};

    #[test]
    fn geometry_uses_integer_emu_and_theme_references() {
        assert_eq!(Emu::inches(1), Emu(914_400));
        assert_eq!(Angle60000(900_000).0, 900_000);
        assert_eq!(
            ColorRef::Theme(ThemeColor::Accent1),
            ColorRef::Theme(ThemeColor::Accent1)
        );
        assert_eq!(
            FontRef::Theme(ThemeFont::Minor),
            FontRef::Theme(ThemeFont::Minor)
        );
        assert!(SlideSize::WIDESCREEN.is_valid());
        assert!(SlideSize::CLASSIC_4_3.is_valid());
    }
}
