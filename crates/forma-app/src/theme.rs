//! Editor appearance is a user preference, independent of scenes and render settings.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Theme {
    #[default]
    Forma,
    Paper,
    Midnight,
    Synthwave,
    Matcha,
}

/// Semantic color tokens. Components read roles, never raw theme values, so
/// every theme maps onto the same hierarchy of surfaces, lines and inks.
///
/// Opaque tokens are `0xRRGGBB`. Tokens documented as translucent are
/// `0xRRGGBBAA` and must be used with `gpui::rgba`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Colors {
    /// Light themes invert elevation cues: shadows soften and wells darken.
    pub dark: bool,
    // Surfaces, back to front.
    /// Window background behind docked panels.
    pub canvas: u32,
    /// Docked chrome: title bar, sidebar, status bar.
    pub surface: u32,
    /// Section headers and inset groups inside a surface.
    pub raised: u32,
    /// Menus, popovers, dialogs and tooltips.
    pub overlay: u32,
    /// Recessed wells: the viewport before its first frame, code, tracks.
    pub sunken: u32,
    /// Translucent viewport clusters (`0xRRGGBBAA`).
    pub hud: u32,
    /// Hairline around viewport clusters (`0xRRGGBBAA`).
    pub hud_border: u32,
    /// Modal backdrop (`0xRRGGBBAA`).
    pub scrim: u32,
    /// Elevation shadow (`0xRRGGBBAA`).
    pub shadow: u32,
    // Controls.
    /// Resting fill of secondary buttons.
    pub control: u32,
    /// Hover fill for ghost and secondary controls and rows.
    pub control_hover: u32,
    /// Pressed fill.
    pub control_active: u32,
    /// Editable value wells.
    pub field: u32,
    pub field_hover: u32,
    // Lines.
    /// Dividers between regions and sections.
    pub hairline: u32,
    /// Control and card outlines.
    pub border: u32,
    pub border_strong: u32,
    // Ink.
    pub text: u32,
    /// Labels and secondary values (at least 7:1 on surfaces, 4.5:1 on
    /// every fill they sit on).
    pub text_muted: u32,
    /// Tertiary text: units, counts, hints and placeholders. Still body
    /// text, so at least 4.5:1 on surfaces, wells and controls.
    pub text_faint: u32,
    // Accent.
    /// Fills: primary actions, toggles, progress.
    pub accent: u32,
    pub accent_hover: u32,
    /// Ink on accent fills.
    pub on_accent: u32,
    /// Accent ink on surfaces and on `accent_soft`.
    pub accent_text: u32,
    /// Selection and active-segment fill.
    pub accent_soft: u32,
    pub accent_soft_hover: u32,
    pub accent_border: u32,
    /// Keyboard focus ring.
    pub focus: u32,
    // Status.
    pub danger: u32,
    pub warning: u32,
    pub success: u32,
    /// Axis strokes in the viewport (X, Y, Z).
    pub axis: [u32; 3],
    /// Axis labels on surfaces (X, Y, Z).
    pub axis_text: [u32; 3],
}

impl Theme {
    pub const ALL: [Self; 5] = [
        Self::Forma,
        Self::Paper,
        Self::Midnight,
        Self::Synthwave,
        Self::Matcha,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Forma => "forma-dark",
            Self::Paper => "paper",
            Self::Midnight => "midnight",
            Self::Synthwave => "synthwave",
            Self::Matcha => "matcha",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Forma => "Forma Dark",
            Self::Paper => "Paper",
            Self::Midnight => "Midnight",
            Self::Synthwave => "Synthwave",
            Self::Matcha => "Matcha",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Forma => "Dark · Classic charcoal and mint",
            Self::Paper => "Light · Soft ivory and ink blue",
            Self::Midnight => "Dark · Deep navy and icy blue",
            Self::Synthwave => "Dark · Electric pink after hours",
            Self::Matcha => "Light · A little green tea for your workspace",
        }
    }

    pub fn colors(self) -> Colors {
        match self {
            Self::Forma => Colors {
                dark: true,
                canvas: 0x131416,
                surface: 0x1b1c1f,
                raised: 0x222326,
                overlay: 0x232428,
                sunken: 0x121315,
                hud: 0x1b1c1fd9,
                hud_border: 0xffffff14,
                scrim: 0x0000008c,
                shadow: 0x00000073,
                control: 0x26272b,
                control_hover: 0x2b2c31,
                control_active: 0x33353a,
                field: 0x242529,
                field_hover: 0x2a2b30,
                hairline: 0x26272b,
                border: 0x303237,
                border_strong: 0x404349,
                text: 0xe8e9ec,
                text_muted: 0xabadb4,
                text_faint: 0x8b8e96,
                accent: 0x6ccfb2,
                accent_hover: 0x7fdac0,
                on_accent: 0x0c201a,
                accent_text: 0x84dcc3,
                accent_soft: 0x213a33,
                accent_soft_hover: 0x26443b,
                accent_border: 0x3b6d5f,
                focus: 0x6ccfb2,
                danger: 0xf2757a,
                warning: 0xe9b45f,
                success: 0x6fd39a,
                axis: [0xf0606b, 0x8bd168, 0x5f9ff5],
                axis_text: [0xf38a91, 0x9fd884, 0x86b5f7],
            },
            Self::Paper => Colors {
                dark: false,
                canvas: 0xe7e4dd,
                surface: 0xf7f6f2,
                raised: 0xefede7,
                overlay: 0xffffff,
                sunken: 0xdedbd3,
                hud: 0xfbfaf7e6,
                hud_border: 0x0000001a,
                scrim: 0x2a26203d,
                shadow: 0x3c32222e,
                control: 0xebe9e3,
                control_hover: 0xe6e3dc,
                control_active: 0xdcd8d0,
                field: 0xedeae4,
                field_hover: 0xe6e3dc,
                hairline: 0xe6e2da,
                border: 0xd8d3c9,
                border_strong: 0xbdb7ab,
                text: 0x22252b,
                text_muted: 0x4f525c,
                text_faint: 0x646870,
                accent: 0x2f63d0,
                accent_hover: 0x2856b8,
                on_accent: 0xffffff,
                accent_text: 0x2553b4,
                accent_soft: 0xdfe8fa,
                accent_soft_hover: 0xd2e0f8,
                accent_border: 0x9fb8e8,
                focus: 0x2f63d0,
                danger: 0xc23b3b,
                warning: 0xa4620c,
                success: 0x2d8653,
                axis: [0xd9414f, 0x3e9a32, 0x2f6ce0],
                axis_text: [0xb72f3d, 0x2e7a26, 0x2a5cc0],
            },
            Self::Midnight => Colors {
                dark: true,
                canvas: 0x0b101b,
                surface: 0x111827,
                raised: 0x172033,
                overlay: 0x162033,
                sunken: 0x0c1220,
                hud: 0x111827d9,
                hud_border: 0xffffff14,
                scrim: 0x02050c99,
                shadow: 0x00000080,
                control: 0x1a2335,
                control_hover: 0x1f2a3f,
                control_active: 0x27344d,
                field: 0x182235,
                field_hover: 0x1e2a40,
                hairline: 0x1b2437,
                border: 0x253149,
                border_strong: 0x36445f,
                text: 0xe4ebf7,
                text_muted: 0x9cabc4,
                text_faint: 0x7d8ba3,
                accent: 0x6fbcff,
                accent_hover: 0x87c8ff,
                on_accent: 0x041220,
                accent_text: 0x8ccbff,
                accent_soft: 0x17304d,
                accent_soft_hover: 0x1c3958,
                accent_border: 0x2e5784,
                focus: 0x6fbcff,
                danger: 0xff8088,
                warning: 0xf2c27a,
                success: 0x74d6a2,
                axis: [0xf06878, 0x7fd38f, 0x6aa8ff],
                axis_text: [0xf490a0, 0x98dca7, 0x92bfff],
            },
            Self::Synthwave => Colors {
                dark: true,
                canvas: 0x130c1d,
                surface: 0x1a1227,
                raised: 0x221832,
                overlay: 0x231833,
                sunken: 0x120b1b,
                hud: 0x1a1227d9,
                hud_border: 0xffffff17,
                scrim: 0x0a041299,
                shadow: 0x05010a80,
                control: 0x261b38,
                control_hover: 0x2c2041,
                control_active: 0x372850,
                field: 0x241a36,
                field_hover: 0x2b1f40,
                hairline: 0x261c37,
                border: 0x342649,
                border_strong: 0x4b3768,
                text: 0xf7ecff,
                text_muted: 0xc0a9d6,
                text_faint: 0x9480aa,
                accent: 0xff6fd2,
                accent_hover: 0xff8adb,
                on_accent: 0x2b0620,
                accent_text: 0xff92de,
                accent_soft: 0x3d1a42,
                accent_soft_hover: 0x48204e,
                accent_border: 0x7d3a70,
                focus: 0xff6fd2,
                danger: 0xff7a8a,
                warning: 0xffc078,
                success: 0x7be0b0,
                axis: [0xff6b8b, 0x6fe3b4, 0x74b8ff],
                axis_text: [0xff92aa, 0x8ee9c3, 0x96c9ff],
            },
            Self::Matcha => Colors {
                dark: false,
                canvas: 0xdbe2d1,
                surface: 0xf2f5ec,
                raised: 0xe9eedf,
                overlay: 0xfbfcf8,
                sunken: 0xd6ddcb,
                hud: 0xf6f8f1e6,
                hud_border: 0x1b2a101f,
                scrim: 0x1e281633,
                shadow: 0x28341e2e,
                control: 0xe5ebdb,
                control_hover: 0xdfe6d3,
                control_active: 0xd3dcc5,
                field: 0xe7ecde,
                field_hover: 0xdfe6d4,
                hairline: 0xe1e7d6,
                border: 0xcfd8c2,
                border_strong: 0xb2bea3,
                text: 0x232d1d,
                text_muted: 0x495440,
                text_faint: 0x636b5b,
                accent: 0x3f7d33,
                accent_hover: 0x356c2b,
                on_accent: 0xffffff,
                accent_text: 0x346b29,
                accent_soft: 0xdae9cc,
                accent_soft_hover: 0xcde0bb,
                accent_border: 0x92b581,
                focus: 0x3f7d33,
                danger: 0xb8403c,
                warning: 0x97600f,
                success: 0x2f7d45,
                axis: [0xd04452, 0x3f8f2c, 0x3a68cc],
                axis_text: [0xa93442, 0x2f6f20, 0x2f56ad],
            },
        }
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(id) => Ok(Self::ALL
                .into_iter()
                .find(|theme| theme.id() == id.trim())
                .unwrap_or_default()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub fn save(self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension(format!(
            "{}-{}.tmp",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            writeln!(file, "{}", self.id())?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&temporary, path)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temporary);
        }
        result
    }
}

/// Resolve platform conventions without changing process environment in tests.
pub(crate) fn preference_path() -> Option<PathBuf> {
    preference_path_for(std::env::consts::OS, |key| {
        std::env::var_os(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    })
}

fn preference_path_for(os: &str, env: impl Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let root = match os {
        "windows" => env("APPDATA")?,
        "macos" => env("HOME")?.join("Library/Application Support"),
        _ => env("XDG_CONFIG_HOME")
            .filter(|path| path.is_absolute())
            .or_else(|| env("HOME").map(|home| home.join(".config")))?,
    };
    Some(root.join("forma").join("color-theme"))
}

/// Browsing changes only the preview. The caller commits explicitly on click/Return.
pub(crate) struct ThemePicker {
    pub query: String,
    pub index: usize,
    original: Theme,
}

impl ThemePicker {
    pub fn new(original: Theme) -> Self {
        Self {
            query: String::new(),
            index: Theme::ALL
                .iter()
                .position(|theme| *theme == original)
                .unwrap_or(0),
            original,
        }
    }

    pub fn matches(&self) -> Vec<Theme> {
        let query = self.query.to_lowercase();
        Theme::ALL
            .into_iter()
            .filter(|theme| {
                let text = format!("{} {}", theme.name(), theme.description()).to_lowercase();
                query.split_whitespace().all(|word| text.contains(word))
            })
            .collect()
    }

    pub fn selected(&self) -> Option<Theme> {
        self.matches().get(self.index).copied()
    }

    pub fn preview(&self) -> Theme {
        self.selected().unwrap_or(self.original)
    }

    pub fn navigate(&mut self, delta: isize) {
        let count = self.matches().len();
        self.index = if count == 0 {
            0
        } else {
            (self.index as isize + delta).rem_euclid(count as isize) as usize
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_selection_stay_readable_in_every_preset() {
        fn luminance(color: u32) -> f64 {
            [16, 8, 0]
                .into_iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(shift, weight)| {
                    let channel = f64::from((color >> shift) & 255) / 255.;
                    weight
                        * if channel <= 0.04045 {
                            channel / 12.92
                        } else {
                            ((channel + 0.055) / 1.055).powf(2.4)
                        }
                })
                .sum()
        }
        let contrast = |ink: u32, background: u32| {
            let (a, b) = (luminance(ink), luminance(background));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        for theme in Theme::ALL {
            let t = theme.colors();
            // Body text, labels and accent ink meet WCAG AA on every surface
            // they are drawn on.
            for (role, ink, background) in [
                ("text/surface", t.text, t.surface),
                ("text/field", t.text, t.field),
                ("text/overlay", t.text, t.overlay),
                ("muted/surface", t.text_muted, t.surface),
                ("muted/raised", t.text_muted, t.raised),
                ("muted/field", t.text_muted, t.field),
                ("muted/overlay", t.text_muted, t.overlay),
                ("muted/control", t.text_muted, t.control),
                ("muted/hover", t.text_muted, t.control_hover),
                ("faint/surface", t.text_faint, t.surface),
                ("faint/raised", t.text_faint, t.raised),
                ("faint/field", t.text_faint, t.field),
                ("faint/control", t.text_faint, t.control),
                ("accent/surface", t.accent_text, t.surface),
                ("accent/selection", t.accent_text, t.accent_soft),
                ("accent/selection hover", t.accent_text, t.accent_soft_hover),
                ("on accent", t.on_accent, t.accent),
                ("text/selection", t.text, t.accent_soft),
            ] {
                let ratio = contrast(ink, background);
                assert!(
                    ratio >= 4.5,
                    "{} {role} contrast is {ratio:.2}",
                    theme.name()
                );
            }
            // Secondary labels keep a clear step above tertiary text.
            let muted = contrast(t.text_muted, t.surface);
            assert!(
                muted >= 7.0,
                "{} muted/surface contrast is {muted:.2}",
                theme.name()
            );
            assert!(
                contrast(t.text_muted, t.text_faint) >= 1.3,
                "{} muted and faint text are too close",
                theme.name()
            );
            // Axis labels and status ink are redundant with shape and need 3:1.
            for (role, ink) in [
                ("x", t.axis_text[0]),
                ("y", t.axis_text[1]),
                ("z", t.axis_text[2]),
                ("danger", t.danger),
                ("warning", t.warning),
                ("success", t.success),
                ("accent fill", t.accent),
            ] {
                let ratio = contrast(ink, t.surface);
                assert!(
                    ratio >= 3.0,
                    "{} {role} contrast is {ratio:.2}",
                    theme.name()
                );
            }
        }
    }

    #[test]
    fn browsing_search_and_empty_results_preserve_the_saved_theme() {
        let mut picker = ThemePicker::new(Theme::Midnight);
        assert_eq!(picker.preview(), Theme::Midnight);
        picker.navigate(1);
        assert_eq!(picker.preview(), Theme::Synthwave);
        picker.query = "LIGHT green".into();
        picker.index = 0;
        assert_eq!(picker.selected(), Some(Theme::Matcha));
        picker.query = "not a theme".into();
        picker.navigate(-1);
        assert_eq!(picker.selected(), None);
        assert_eq!(picker.preview(), Theme::Midnight);
        picker.query.clear();
        picker.navigate(-1);
        assert_eq!(picker.preview(), Theme::Matcha);
    }

    #[test]
    fn preferences_round_trip_replace_and_recover_from_unknown_ids() {
        let root = std::env::temp_dir().join(format!("forma-theme-test-{}", std::process::id()));
        let path = root.join("settings/color-theme");
        assert_eq!(Theme::load(&path).unwrap(), Theme::Forma);
        for theme in Theme::ALL {
            theme.save(&path).unwrap();
            assert_eq!(Theme::load(&path).unwrap(), theme);
        }
        std::fs::write(&path, "a future or corrupt theme").unwrap();
        assert_eq!(Theme::load(&path).unwrap(), Theme::Forma);
        assert!(Theme::Synthwave.save(&path.join("impossible")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn preferences_follow_platform_config_directories() {
        // PathBuf uses the host's path rules even when exercising another OS's
        // preference convention. A Unix "/config" path is not absolute on Windows.
        let root = std::env::current_dir().unwrap().join("preference-fixture");
        let home = root.join("users/test");
        let config = root.join("config");
        let appdata = home.join("AppData/Roaming");
        let env = |key: &str| match key {
            "APPDATA" => Some(appdata.clone()),
            "HOME" => Some(home.clone()),
            "XDG_CONFIG_HOME" => Some(config.clone()),
            _ => None,
        };
        assert_eq!(
            preference_path_for("windows", env).unwrap(),
            appdata.join("forma/color-theme")
        );
        assert_eq!(
            preference_path_for("macos", env).unwrap(),
            home.join("Library/Application Support/forma/color-theme")
        );
        assert_eq!(
            preference_path_for("linux", env).unwrap(),
            config.join("forma/color-theme")
        );
        assert_eq!(
            preference_path_for("linux", |key| (key == "HOME").then(|| home.clone())).unwrap(),
            home.join(".config/forma/color-theme")
        );
        assert_eq!(
            preference_path_for("linux", |key| match key {
                "HOME" => Some(home.clone()),
                "XDG_CONFIG_HOME" => Some(PathBuf::from("relative/config")),
                _ => None,
            })
            .unwrap(),
            home.join(".config/forma/color-theme")
        );
        assert!(preference_path_for("linux", |_| None).is_none());
    }
}
