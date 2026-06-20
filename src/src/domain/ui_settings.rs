pub const DEFAULT_WINDOW_WIDTH: i32 = 900;
pub const DEFAULT_WINDOW_HEIGHT: i32 = 600;
pub const DEFAULT_SPLITTER_LEFT_WIDTH: i32 = 280;
pub const SETTING_WINDOW_X: &str = "window.x";
pub const SETTING_WINDOW_Y: &str = "window.y";
pub const SETTING_WINDOW_WIDTH: &str = "window.width";
pub const SETTING_WINDOW_HEIGHT: &str = "window.height";
pub const SETTING_SPLITTER_LEFT_WIDTH: &str = "splitter.left_width";
pub const SETTING_SELECTION_NODE_ID: &str = "selection.node_id";
pub const SETTING_EDITOR_FONT_FAMILY: &str = "editor.font.family";
pub const SETTING_EDITOR_FONT_SIZE_PT: &str = "editor.font.size_pt";
pub const SETTING_EDITOR_WORD_WRAP: &str = "editor.word_wrap";
pub const SETTING_TEXT_IMPORT_ENCODING: &str = "text.import.encoding";
pub const SETTING_TEXT_EXPORT_ENCODING: &str = "text.export.encoding";
pub const SETTING_APPEARANCE_THEME: &str = "appearance.theme";
pub const SETTING_APPEARANCE_DARK_THEME: &str = "appearance.dark_theme";
pub const SETTING_UI_LANGUAGE: &str = "ui.language";
pub const SETTING_AUTO_SAVE_ENABLED: &str = "autosave.enabled";
pub const SETTING_AUTO_SAVE_INTERVAL_SECONDS: &str = "autosave.interval_seconds";
pub const DEFAULT_EDITOR_FONT_FAMILY: &str = "Consolas";
pub const DEFAULT_EDITOR_FONT_SIZE_PT: i32 = 10;
pub const DEFAULT_EDITOR_WORD_WRAP: bool = true;
pub const DEFAULT_APPEARANCE_THEME: AppearanceTheme = AppearanceTheme::Light;
pub const DEFAULT_UI_LANGUAGE: UiLanguage = UiLanguage::English;
pub const DEFAULT_DARK_THEME: bool = false;
pub const DEFAULT_AUTO_SAVE_ENABLED: bool = false;
pub const DEFAULT_AUTO_SAVE_INTERVAL_SECONDS: i32 = 120;
pub const MIN_EDITOR_FONT_SIZE_PT: i32 = 6;
pub const MAX_EDITOR_FONT_SIZE_PT: i32 = 72;

const MIN_WINDOW_WIDTH: i32 = 320;
const MIN_WINDOW_HEIGHT: i32 = 240;
const MAX_WINDOW_WIDTH: i32 = 20_000;
const MAX_WINDOW_HEIGHT: i32 = 20_000;
const MIN_SPLITTER_LEFT_WIDTH: i32 = 80;
const MAX_SPLITTER_LEFT_WIDTH: i32 = 20_000;
const MIN_AUTO_SAVE_INTERVAL_SECONDS: i32 = 15;
const MAX_AUTO_SAVE_INTERVAL_SECONDS: i32 = 3_600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiLanguage {
    Korean,
    English,
}

const UI_LANGUAGES: [UiLanguage; 2] = [UiLanguage::Korean, UiLanguage::English];

impl UiLanguage {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Korean => "한국어",
            Self::English => "English",
        }
    }

    pub fn storage_value(self) -> &'static str {
        match self {
            Self::Korean => "ko",
            Self::English => "en",
        }
    }

    pub fn from_storage_value(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "ko" | "ko-kr" | "korean" | "한국어" => Some(Self::Korean),
            "en" | "en-us" | "en-gb" | "english" => Some(Self::English),
            _ => None,
        }
    }

    pub fn options() -> &'static [Self] {
        &UI_LANGUAGES
    }
}

impl Default for UiLanguage {
    fn default() -> Self {
        DEFAULT_UI_LANGUAGE
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextEncoding {
    AutoDetect,
    Utf8,
    Utf8WithBom,
    Utf16LeWithBom,
    Utf16BeWithBom,
    KoreanEucKr,
    Windows1252,
}

const TEXT_IMPORT_ENCODINGS: [TextEncoding; 7] = [
    TextEncoding::AutoDetect,
    TextEncoding::Utf8,
    TextEncoding::Utf8WithBom,
    TextEncoding::Utf16LeWithBom,
    TextEncoding::Utf16BeWithBom,
    TextEncoding::KoreanEucKr,
    TextEncoding::Windows1252,
];

const TEXT_EXPORT_ENCODINGS: [TextEncoding; 6] = [
    TextEncoding::Utf8,
    TextEncoding::Utf8WithBom,
    TextEncoding::Utf16LeWithBom,
    TextEncoding::Utf16BeWithBom,
    TextEncoding::KoreanEucKr,
    TextEncoding::Windows1252,
];

impl TextEncoding {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::AutoDetect => "자동 감지",
            Self::Utf8 => "UTF-8",
            Self::Utf8WithBom => "UTF-8 BOM",
            Self::Utf16LeWithBom => "UTF-16 LE BOM",
            Self::Utf16BeWithBom => "UTF-16 BE BOM",
            Self::KoreanEucKr => "한국어 (EUC-KR/CP949)",
            Self::Windows1252 => "Windows-1252",
        }
    }

    pub fn storage_value(self) -> &'static str {
        match self {
            Self::AutoDetect => "auto",
            Self::Utf8 => "utf-8",
            Self::Utf8WithBom => "utf-8-bom",
            Self::Utf16LeWithBom => "utf-16le-bom",
            Self::Utf16BeWithBom => "utf-16be-bom",
            Self::KoreanEucKr => "euc-kr",
            Self::Windows1252 => "windows-1252",
        }
    }

    pub fn from_storage_value(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "auto" | "auto-detect" | "auto_detect" => Some(Self::AutoDetect),
            "utf-8" | "utf8" => Some(Self::Utf8),
            "utf-8-bom" | "utf8-bom" | "utf-8-with-bom" => Some(Self::Utf8WithBom),
            "utf-16le-bom" | "utf-16-le-bom" | "utf16le-bom" => Some(Self::Utf16LeWithBom),
            "utf-16be-bom" | "utf-16-be-bom" | "utf16be-bom" => Some(Self::Utf16BeWithBom),
            "euc-kr" | "windows-949" | "windows949" | "cp949" => Some(Self::KoreanEucKr),
            "windows-1252" | "windows1252" | "cp1252" => Some(Self::Windows1252),
            _ => None,
        }
    }

    pub fn from_import_storage_value(value: &str) -> Option<Self> {
        Self::from_storage_value(value).filter(|encoding| encoding.is_import_supported())
    }

    pub fn from_export_storage_value(value: &str) -> Option<Self> {
        Self::from_storage_value(value).filter(|encoding| encoding.is_export_supported())
    }

    pub fn default_import() -> Self {
        Self::AutoDetect
    }

    pub fn default_export() -> Self {
        Self::Utf8
    }

    pub fn import_options() -> &'static [Self] {
        &TEXT_IMPORT_ENCODINGS
    }

    pub fn export_options() -> &'static [Self] {
        &TEXT_EXPORT_ENCODINGS
    }

    pub fn is_import_supported(self) -> bool {
        TEXT_IMPORT_ENCODINGS.contains(&self)
    }

    pub fn is_export_supported(self) -> bool {
        TEXT_EXPORT_ENCODINGS.contains(&self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextEncodingSettings {
    pub import_encoding: TextEncoding,
    pub export_encoding: TextEncoding,
}

impl Default for TextEncodingSettings {
    fn default() -> Self {
        Self {
            import_encoding: TextEncoding::default_import(),
            export_encoding: TextEncoding::default_export(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UiSettings {
    pub window: WindowSettings,
    pub splitter: SplitterSettings,
    pub selection: SelectionSettings,
    pub editor_font: EditorFontSettings,
    pub editor: EditorSettings,
    pub text_encoding: TextEncodingSettings,
    pub appearance: AppearanceSettings,
    pub language: UiLanguage,
    pub auto_save: AutoSaveSettings,
}

impl UiSettings {
    pub fn from_entries<'a>(entries: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut settings = Self::default();
        let mut appearance_theme_seen = false;

        for (key, value) in entries {
            match key {
                SETTING_WINDOW_X => {
                    settings.window.x = parse_i32(value);
                }
                SETTING_WINDOW_Y => {
                    settings.window.y = parse_i32(value);
                }
                SETTING_WINDOW_WIDTH => {
                    settings.window.width =
                        parse_bounded_i32(value, MIN_WINDOW_WIDTH, MAX_WINDOW_WIDTH)
                            .unwrap_or(DEFAULT_WINDOW_WIDTH);
                }
                SETTING_WINDOW_HEIGHT => {
                    settings.window.height =
                        parse_bounded_i32(value, MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT)
                            .unwrap_or(DEFAULT_WINDOW_HEIGHT);
                }
                SETTING_SPLITTER_LEFT_WIDTH => {
                    settings.splitter.left_width =
                        parse_bounded_i32(value, MIN_SPLITTER_LEFT_WIDTH, MAX_SPLITTER_LEFT_WIDTH)
                            .unwrap_or(DEFAULT_SPLITTER_LEFT_WIDTH);
                }
                SETTING_SELECTION_NODE_ID => {
                    settings.selection.node_id = parse_i64(value).filter(|node_id| *node_id > 0);
                }
                SETTING_EDITOR_FONT_FAMILY => {
                    settings.editor_font.family = normalize_editor_font_family(value);
                }
                SETTING_EDITOR_FONT_SIZE_PT => {
                    settings.editor_font.size_pt =
                        parse_bounded_i32(value, MIN_EDITOR_FONT_SIZE_PT, MAX_EDITOR_FONT_SIZE_PT)
                            .unwrap_or(DEFAULT_EDITOR_FONT_SIZE_PT);
                }
                SETTING_EDITOR_WORD_WRAP => {
                    settings.editor.word_wrap =
                        parse_editor_word_wrap(value).unwrap_or(DEFAULT_EDITOR_WORD_WRAP);
                }
                SETTING_TEXT_IMPORT_ENCODING => {
                    settings.text_encoding.import_encoding =
                        TextEncoding::from_import_storage_value(value)
                            .unwrap_or_else(TextEncoding::default_import);
                }
                SETTING_TEXT_EXPORT_ENCODING => {
                    settings.text_encoding.export_encoding =
                        TextEncoding::from_export_storage_value(value)
                            .unwrap_or_else(TextEncoding::default_export);
                }
                SETTING_APPEARANCE_THEME => {
                    appearance_theme_seen = true;
                    settings.appearance.theme =
                        AppearanceTheme::from_storage_value(value).unwrap_or_default();
                }
                SETTING_APPEARANCE_DARK_THEME if !appearance_theme_seen => {
                    settings.appearance.theme = parse_dark_theme(value)
                        .map(AppearanceTheme::from_legacy_dark_theme)
                        .unwrap_or_default();
                }
                SETTING_UI_LANGUAGE => {
                    settings.language = UiLanguage::from_storage_value(value).unwrap_or_default();
                }
                SETTING_AUTO_SAVE_ENABLED => {
                    settings.auto_save.enabled =
                        parse_auto_save_enabled(value).unwrap_or(DEFAULT_AUTO_SAVE_ENABLED);
                }
                SETTING_AUTO_SAVE_INTERVAL_SECONDS => {
                    settings.auto_save.interval_seconds = parse_bounded_i32(
                        value,
                        MIN_AUTO_SAVE_INTERVAL_SECONDS,
                        MAX_AUTO_SAVE_INTERVAL_SECONDS,
                    )
                    .unwrap_or(DEFAULT_AUTO_SAVE_INTERVAL_SECONDS);
                }
                _ => {}
            }
        }

        settings
    }

    pub fn entries(&self) -> Vec<(&'static str, String)> {
        let mut entries = Vec::with_capacity(16);
        if let Some(x) = self.window.x {
            entries.push((SETTING_WINDOW_X, x.to_string()));
        }
        if let Some(y) = self.window.y {
            entries.push((SETTING_WINDOW_Y, y.to_string()));
        }
        entries.extend([
            (SETTING_WINDOW_WIDTH, self.window.width.to_string()),
            (SETTING_WINDOW_HEIGHT, self.window.height.to_string()),
            (
                SETTING_SPLITTER_LEFT_WIDTH,
                self.splitter.left_width.to_string(),
            ),
            (
                SETTING_SELECTION_NODE_ID,
                self.selection.node_id.unwrap_or_default().to_string(),
            ),
            (SETTING_EDITOR_FONT_FAMILY, self.editor_font.family.clone()),
            (
                SETTING_EDITOR_FONT_SIZE_PT,
                self.editor_font.size_pt.to_string(),
            ),
            (
                SETTING_EDITOR_WORD_WRAP,
                editor_word_wrap_storage_value(self.editor.word_wrap).to_owned(),
            ),
            (
                SETTING_TEXT_IMPORT_ENCODING,
                self.text_encoding
                    .import_encoding
                    .storage_value()
                    .to_owned(),
            ),
            (
                SETTING_TEXT_EXPORT_ENCODING,
                self.text_encoding
                    .export_encoding
                    .storage_value()
                    .to_owned(),
            ),
            (
                SETTING_APPEARANCE_THEME,
                self.appearance.theme.storage_value().to_owned(),
            ),
            (
                SETTING_APPEARANCE_DARK_THEME,
                dark_theme_storage_value(self.appearance.theme.uses_dark_mode()).to_owned(),
            ),
            (
                SETTING_UI_LANGUAGE,
                self.language.storage_value().to_owned(),
            ),
            (
                SETTING_AUTO_SAVE_ENABLED,
                auto_save_enabled_storage_value(self.auto_save.enabled).to_owned(),
            ),
            (
                SETTING_AUTO_SAVE_INTERVAL_SECONDS,
                self.auto_save.interval_seconds.to_string(),
            ),
        ]);
        entries
    }

    pub fn changed_entries(&self, previous: &Self) -> Vec<(&'static str, String)> {
        let mut entries = Vec::new();
        if self.window.x != previous.window.x {
            if let Some(x) = self.window.x {
                entries.push((SETTING_WINDOW_X, x.to_string()));
            }
        }
        if self.window.y != previous.window.y {
            if let Some(y) = self.window.y {
                entries.push((SETTING_WINDOW_Y, y.to_string()));
            }
        }
        if self.window.width != previous.window.width {
            entries.push((SETTING_WINDOW_WIDTH, self.window.width.to_string()));
        }
        if self.window.height != previous.window.height {
            entries.push((SETTING_WINDOW_HEIGHT, self.window.height.to_string()));
        }
        if self.splitter.left_width != previous.splitter.left_width {
            entries.push((
                SETTING_SPLITTER_LEFT_WIDTH,
                self.splitter.left_width.to_string(),
            ));
        }
        if self.selection.node_id != previous.selection.node_id {
            entries.push((
                SETTING_SELECTION_NODE_ID,
                self.selection.node_id.unwrap_or_default().to_string(),
            ));
        }
        if self.editor_font.family != previous.editor_font.family {
            entries.push((SETTING_EDITOR_FONT_FAMILY, self.editor_font.family.clone()));
        }
        if self.editor_font.size_pt != previous.editor_font.size_pt {
            entries.push((
                SETTING_EDITOR_FONT_SIZE_PT,
                self.editor_font.size_pt.to_string(),
            ));
        }
        if self.editor.word_wrap != previous.editor.word_wrap {
            entries.push((
                SETTING_EDITOR_WORD_WRAP,
                editor_word_wrap_storage_value(self.editor.word_wrap).to_owned(),
            ));
        }
        if self.text_encoding.import_encoding != previous.text_encoding.import_encoding {
            entries.push((
                SETTING_TEXT_IMPORT_ENCODING,
                self.text_encoding
                    .import_encoding
                    .storage_value()
                    .to_owned(),
            ));
        }
        if self.text_encoding.export_encoding != previous.text_encoding.export_encoding {
            entries.push((
                SETTING_TEXT_EXPORT_ENCODING,
                self.text_encoding
                    .export_encoding
                    .storage_value()
                    .to_owned(),
            ));
        }
        if self.appearance.theme != previous.appearance.theme {
            entries.push((
                SETTING_APPEARANCE_THEME,
                self.appearance.theme.storage_value().to_owned(),
            ));
            entries.push((
                SETTING_APPEARANCE_DARK_THEME,
                dark_theme_storage_value(self.appearance.theme.uses_dark_mode()).to_owned(),
            ));
        }
        if self.language != previous.language {
            entries.push((
                SETTING_UI_LANGUAGE,
                self.language.storage_value().to_owned(),
            ));
        }
        if self.auto_save.enabled != previous.auto_save.enabled {
            entries.push((
                SETTING_AUTO_SAVE_ENABLED,
                auto_save_enabled_storage_value(self.auto_save.enabled).to_owned(),
            ));
        }
        if self.auto_save.interval_seconds != previous.auto_save.interval_seconds {
            entries.push((
                SETTING_AUTO_SAVE_INTERVAL_SECONDS,
                self.auto_save.interval_seconds.to_string(),
            ));
        }
        entries
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppearanceSettings {
    pub theme: AppearanceTheme,
}

impl AppearanceSettings {
    pub fn set_theme(&mut self, theme: AppearanceTheme) {
        self.theme = theme;
    }
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: DEFAULT_APPEARANCE_THEME,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppearanceTheme {
    Light,
    ClassicDark,
    SepiaTeal,
    Graphite,
    Forest,
    SteelBlue,
}

const APPEARANCE_THEMES: [AppearanceTheme; 6] = [
    AppearanceTheme::Light,
    AppearanceTheme::ClassicDark,
    AppearanceTheme::SepiaTeal,
    AppearanceTheme::Graphite,
    AppearanceTheme::Forest,
    AppearanceTheme::SteelBlue,
];

impl AppearanceTheme {
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Light => "밝게",
            Self::ClassicDark => "어둡게",
            Self::SepiaTeal => "세피아",
            Self::Graphite => "그래파이트",
            Self::Forest => "숲",
            Self::SteelBlue => "스틸 블루",
        }
    }

    pub fn storage_value(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::ClassicDark => "classic-dark",
            Self::SepiaTeal => "sepia-teal",
            Self::Graphite => "graphite",
            Self::Forest => "forest",
            Self::SteelBlue => "steel-blue",
        }
    }

    pub fn from_storage_value(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "light" => Some(Self::Light),
            "dark" | "classic-dark" | "classic_dark" => Some(Self::ClassicDark),
            "sepia-teal" | "sepia_teal" | "sepia" => Some(Self::SepiaTeal),
            "graphite" | "gray" | "grey" => Some(Self::Graphite),
            "forest" | "green" => Some(Self::Forest),
            "steel-blue" | "steel_blue" | "steel" => Some(Self::SteelBlue),
            _ => None,
        }
    }

    pub fn options() -> &'static [Self] {
        &APPEARANCE_THEMES
    }

    pub fn uses_dark_mode(self) -> bool {
        self != Self::Light
    }

    fn from_legacy_dark_theme(enabled: bool) -> Self {
        if enabled {
            Self::ClassicDark
        } else {
            Self::Light
        }
    }
}

impl Default for AppearanceTheme {
    fn default() -> Self {
        DEFAULT_APPEARANCE_THEME
    }
}

pub fn dark_theme_storage_value(enabled: bool) -> &'static str {
    if enabled {
        "true"
    } else {
        "false"
    }
}

fn parse_dark_theme(value: &str) -> Option<bool> {
    parse_storage_bool(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EditorSettings {
    pub word_wrap: bool,
}

impl EditorSettings {
    pub fn toggle_word_wrap(&mut self) {
        self.word_wrap = toggle_editor_word_wrap(self.word_wrap);
    }
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            word_wrap: DEFAULT_EDITOR_WORD_WRAP,
        }
    }
}

pub fn toggle_editor_word_wrap(enabled: bool) -> bool {
    !enabled
}

pub fn editor_word_wrap_storage_value(enabled: bool) -> &'static str {
    if enabled {
        "true"
    } else {
        "false"
    }
}

fn parse_editor_word_wrap(value: &str) -> Option<bool> {
    parse_storage_bool(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoSaveSettings {
    pub enabled: bool,
    pub interval_seconds: i32,
}

impl AutoSaveSettings {
    pub fn new(enabled: bool, interval_seconds: i32) -> Self {
        Self {
            enabled,
            interval_seconds: interval_seconds.clamp(
                MIN_AUTO_SAVE_INTERVAL_SECONDS,
                MAX_AUTO_SAVE_INTERVAL_SECONDS,
            ),
        }
    }
}

impl Default for AutoSaveSettings {
    fn default() -> Self {
        Self {
            enabled: DEFAULT_AUTO_SAVE_ENABLED,
            interval_seconds: DEFAULT_AUTO_SAVE_INTERVAL_SECONDS,
        }
    }
}

pub fn auto_save_enabled_storage_value(enabled: bool) -> &'static str {
    if enabled {
        "true"
    } else {
        "false"
    }
}

fn parse_auto_save_enabled(value: &str) -> Option<bool> {
    parse_storage_bool(value)
}

fn parse_storage_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorFontSettings {
    pub family: String,
    pub size_pt: i32,
}

impl EditorFontSettings {
    pub fn new(family: impl AsRef<str>, size_pt: i32) -> Self {
        Self {
            family: normalize_editor_font_family(family.as_ref()),
            size_pt: size_pt.clamp(MIN_EDITOR_FONT_SIZE_PT, MAX_EDITOR_FONT_SIZE_PT),
        }
    }
}

impl Default for EditorFontSettings {
    fn default() -> Self {
        Self {
            family: DEFAULT_EDITOR_FONT_FAMILY.to_owned(),
            size_pt: DEFAULT_EDITOR_FONT_SIZE_PT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSettings {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: i32,
    pub height: i32,
}

impl WindowSettings {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x: Some(x),
            y: Some(y),
            width: width.clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_WIDTH),
            height: height.clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT),
        }
    }

    pub fn with_size(self, width: i32, height: i32) -> Self {
        Self {
            width: width.clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_WIDTH),
            height: height.clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT),
            ..self
        }
    }
}

impl Default for WindowSettings {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: DEFAULT_WINDOW_WIDTH,
            height: DEFAULT_WINDOW_HEIGHT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitterSettings {
    pub left_width: i32,
}

impl SplitterSettings {
    pub fn new(left_width: i32) -> Self {
        Self {
            left_width: left_width.clamp(MIN_SPLITTER_LEFT_WIDTH, MAX_SPLITTER_LEFT_WIDTH),
        }
    }
}

impl Default for SplitterSettings {
    fn default() -> Self {
        Self {
            left_width: DEFAULT_SPLITTER_LEFT_WIDTH,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SelectionSettings {
    pub node_id: Option<i64>,
}

fn parse_i32(value: &str) -> Option<i32> {
    value.trim().parse::<i32>().ok()
}

fn parse_i64(value: &str) -> Option<i64> {
    value.trim().parse::<i64>().ok()
}

fn parse_bounded_i32(value: &str, min: i32, max: i32) -> Option<i32> {
    parse_i32(value).filter(|parsed| *parsed >= min && *parsed <= max)
}

fn normalize_editor_font_family(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains('\0') {
        DEFAULT_EDITOR_FONT_FAMILY.to_owned()
    } else {
        trimmed.to_owned()
    }
}
