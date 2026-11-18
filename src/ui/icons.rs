use std::path::PathBuf;

/// Fluent UI System Icons used by the first native commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemIcon {
    Open,
    Save,
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconSize {
    Px16,
    Px20,
    Px24,
}

impl IconSize {
    const fn pixels(self) -> u8 {
        match self {
            Self::Px16 => 16,
            Self::Px20 => 20,
            Self::Px24 => 24,
        }
    }

    const ALL: [Self; 3] = [Self::Px16, Self::Px20, Self::Px24];
}

impl SystemIcon {
    const ALL: [Self; 4] = [Self::Open, Self::Save, Self::Undo, Self::Redo];

    const fn stem(self) -> &'static str {
        match self {
            Self::Open => "folder_open",
            Self::Save => "save",
            Self::Undo => "arrow_undo",
            Self::Redo => "arrow_redo",
        }
    }

    /// Resolves a bundled Microsoft Fluent System Icon from the project assets.
    pub fn asset_path(self, size: IconSize) -> PathBuf {
        PathBuf::from("assets").join("icons").join(format!(
            "{}_{}_regular.svg",
            self.stem(),
            size.pixels()
        ))
    }

    /// Returns the source SVG embedded from the checked-in Microsoft icon bundle.
    pub fn source(self, size: IconSize) -> &'static str {
        match (self, size) {
            (Self::Open, IconSize::Px16) => {
                include_str!("../../assets/icons/folder_open_16_regular.svg")
            }
            (Self::Open, IconSize::Px20) => {
                include_str!("../../assets/icons/folder_open_20_regular.svg")
            }
            (Self::Open, IconSize::Px24) => {
                include_str!("../../assets/icons/folder_open_24_regular.svg")
            }
            (Self::Save, IconSize::Px16) => include_str!("../../assets/icons/save_16_regular.svg"),
            (Self::Save, IconSize::Px20) => include_str!("../../assets/icons/save_20_regular.svg"),
            (Self::Save, IconSize::Px24) => include_str!("../../assets/icons/save_24_regular.svg"),
            (Self::Undo, IconSize::Px16) => {
                include_str!("../../assets/icons/arrow_undo_16_regular.svg")
            }
            (Self::Undo, IconSize::Px20) => {
                include_str!("../../assets/icons/arrow_undo_20_regular.svg")
            }
            (Self::Undo, IconSize::Px24) => {
                include_str!("../../assets/icons/arrow_undo_24_regular.svg")
            }
            (Self::Redo, IconSize::Px16) => {
                include_str!("../../assets/icons/arrow_redo_16_regular.svg")
            }
            (Self::Redo, IconSize::Px20) => {
                include_str!("../../assets/icons/arrow_redo_20_regular.svg")
            }
            (Self::Redo, IconSize::Px24) => {
                include_str!("../../assets/icons/arrow_redo_24_regular.svg")
            }
        }
    }
}

/// Confirms that all embedded icon files are valid SVGs with their native view boxes.
pub fn validate_bundled_icons() -> Result<(), crate::AppError> {
    for icon in SystemIcon::ALL {
        for size in IconSize::ALL {
            let source = icon.source(size);
            let document = roxmltree::Document::parse(source).map_err(|error| {
                crate::AppError::IconAsset(format!("{}: {error}", icon.asset_path(size).display()))
            })?;
            let root = document.root_element();
            let expected_view_box = format!("0 0 {} {}", size.pixels(), size.pixels());
            if root.tag_name().name() != "svg"
                || root.attribute("viewBox") != Some(expected_view_box.as_str())
            {
                return Err(crate::AppError::IconAsset(
                    icon.asset_path(size).display().to_string(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn all_fluent_command_icons_are_embedded_at_native_sizes() {
        assert!(super::validate_bundled_icons().is_ok());
    }
}
