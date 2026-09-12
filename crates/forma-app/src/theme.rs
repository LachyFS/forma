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

#[derive(Clone, Copy)]
pub(crate) struct Colors {
    pub shell: u32,
    pub panel: u32,
    pub card: u32,
    pub row_alt: u32,
    pub raised: u32,
    pub input: u32,
    pub input_hover: u32,
    pub well: u32,
    pub line: u32,
    pub edge: u32,
    pub text: u32,
    pub muted: u32,
    pub faint: u32,
    pub accent: u32,
    pub accent_line: u32,
    pub active: u32,
    pub active_hover: u32,
    pub alert: u32,
    pub axis_ink: [u32; 3],
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
                shell: 0x1d1e20,
                panel: 0x292a2d,
                card: 0x333437,
                row_alt: 0x2c2d30,
                raised: 0x3c3e42,
                input: 0x46484c,
                input_hover: 0x53565b,
                well: 0x222326,
                line: 0x1c1d1f,
                edge: 0x4b4d51,
                text: 0xe1e2e4,
                muted: 0xa5a7ad,
                faint: 0x9699a1,
                accent: 0x96d5c2,
                accent_line: 0x597d71,
                active: 0x354e46,
                active_hover: 0x3b594f,
                alert: 0xd7a175,
                axis_ink: [0xe0a0a1, 0xa7cfb1, 0xa5bde9],
            },
            Self::Paper => Colors {
                shell: 0xdedbd5,
                panel: 0xf7f5ef,
                card: 0xedeae2,
                row_alt: 0xf0eee7,
                raised: 0xe2ded5,
                input: 0xffffff,
                input_hover: 0xf2f0e9,
                well: 0xe5e2db,
                line: 0xd3cfc5,
                edge: 0xc4beb2,
                text: 0x292d36,
                muted: 0x535967,
                faint: 0x626571,
                accent: 0x255bac,
                accent_line: 0x7a9bc8,
                active: 0xd8e5f6,
                active_hover: 0xc9dcf4,
                alert: 0x88501b,
                axis_ink: [0xa53245, 0x306640, 0x315db0],
            },
            Self::Midnight => Colors {
                shell: 0x101624,
                panel: 0x192235,
                card: 0x222e44,
                row_alt: 0x1e293d,
                raised: 0x2c3b54,
                input: 0x34445e,
                input_hover: 0x40516c,
                well: 0x131c2d,
                line: 0x101827,
                edge: 0x405371,
                text: 0xe2eaf7,
                muted: 0xa8bad5,
                faint: 0x94a9c7,
                accent: 0x8dccff,
                accent_line: 0x527da4,
                active: 0x233f5b,
                active_hover: 0x2d506f,
                alert: 0xf0bc83,
                axis_ink: [0xf2a3b3, 0x9bd7b5, 0xa4c5ff],
            },
            Self::Synthwave => Colors {
                shell: 0x1c132b,
                panel: 0x281d3b,
                card: 0x35264b,
                row_alt: 0x2e2242,
                raised: 0x44325e,
                input: 0x4b3764,
                input_hover: 0x584275,
                well: 0x21162f,
                line: 0x180f25,
                edge: 0x674b82,
                text: 0xf5e8ff,
                muted: 0xc8afd9,
                faint: 0xbca1cf,
                accent: 0xff9ddc,
                accent_line: 0xa96399,
                active: 0x57304f,
                active_hover: 0x683c60,
                alert: 0xffc48c,
                axis_ink: [0xffa1c0, 0x8ee4c4, 0x9dcfff],
            },
            Self::Matcha => Colors {
                shell: 0xcfd7c7,
                panel: 0xeff2e6,
                card: 0xe3e9d7,
                row_alt: 0xe8eddd,
                raised: 0xd4dfc6,
                input: 0xfafbef,
                input_hover: 0xf0f5e5,
                well: 0xdce3d0,
                line: 0xc3ccb5,
                edge: 0xabbba0,
                text: 0x2c3929,
                muted: 0x4f6048,
                faint: 0x5d6a52,
                accent: 0x38612e,
                accent_line: 0x829d6d,
                active: 0xd0e0bd,
                active_hover: 0xc2d6ad,
                alert: 0x855024,
                axis_ink: [0xa43b44, 0x326534, 0x3e5ba3],
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
        for theme in Theme::ALL {
            let t = theme.colors();
            for (ink, background) in [
                (t.text, t.input),
                (t.text, t.panel),
                (t.muted, t.panel),
                (t.muted, t.card),
                (t.faint, t.panel),
                (t.accent, t.active),
                (t.accent, t.active_hover),
            ] {
                let (a, b) = (luminance(ink), luminance(background));
                let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                assert!(
                    contrast >= 4.5,
                    "{} text contrast is {contrast:.2}",
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
        let env = |key: &str| match key {
            "APPDATA" => Some(PathBuf::from("C:/Users/test/AppData/Roaming")),
            "HOME" => Some(PathBuf::from("/users/test")),
            "XDG_CONFIG_HOME" => Some(PathBuf::from("/config")),
            _ => None,
        };
        assert_eq!(
            preference_path_for("windows", env).unwrap(),
            PathBuf::from("C:/Users/test/AppData/Roaming/forma/color-theme")
        );
        assert_eq!(
            preference_path_for("macos", env).unwrap(),
            PathBuf::from("/users/test/Library/Application Support/forma/color-theme")
        );
        assert_eq!(
            preference_path_for("linux", env).unwrap(),
            PathBuf::from("/config/forma/color-theme")
        );
        assert_eq!(
            preference_path_for("linux", |key| (key == "HOME")
                .then(|| PathBuf::from("/users/test")))
            .unwrap(),
            PathBuf::from("/users/test/.config/forma/color-theme")
        );
        assert!(preference_path_for("linux", |_| None).is_none());
    }
}
