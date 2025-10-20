use eframe::egui;
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use font_kit::family_name::FamilyName;
use font_kit::properties::Properties;
use font_kit::source::SystemSource;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Issue {
    id: String,
    title: String,
    description: String,
    status: String,
    priority: i32,
    issue_type: String,
    #[serde(default)]
    assignee: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    dependencies: Vec<Issue>,
    #[serde(default)]
    source_directory: String,
}

// Configuration for a single monitored directory
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DirectoryConfig {
    path: PathBuf,
    visible: bool,
    #[serde(default)]
    display_name: String,
}

// Application configuration persisted to ~/.config/beadui/config.yaml
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppConfig {
    #[serde(default)]
    directories: Vec<DirectoryConfig>,
    #[serde(default)]
    sidebar_collapsed: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            directories: Vec::new(),
            sidebar_collapsed: false,
        }
    }
}

impl AppConfig {
    /// Get the path to the config file: ~/.config/beadui/config.yaml
    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|mut path| {
            path.push("beadui");
            path.push("config.yaml");
            path
        })
    }

    /// Load config from ~/.config/beadui/config.yaml
    /// Returns default config if file doesn't exist or is corrupt
    fn load() -> Self {
        let config_path = match Self::config_path() {
            Some(path) => path,
            None => return Self::default(),
        };

        // If file doesn't exist, return default
        if !config_path.exists() {
            return Self::default();
        }

        // Try to read and parse the file
        match fs::read_to_string(&config_path) {
            Ok(contents) => {
                match serde_yaml::from_str::<AppConfig>(&contents) {
                    Ok(config) => config,
                    Err(_) => {
                        // Corrupt file - return default
                        Self::default()
                    }
                }
            }
            Err(_) => Self::default(),
        }
    }

    /// Save config to ~/.config/beadui/config.yaml
    /// Creates directory if it doesn't exist
    fn save(&self) -> Result<(), String> {
        let config_path = Self::config_path()
            .ok_or_else(|| "Could not determine config directory".to_string())?;

        // Create parent directory if it doesn't exist
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config directory: {}", e))?;
        }

        // Serialize to YAML
        let yaml = serde_yaml::to_string(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        // Write to file
        fs::write(&config_path, yaml)
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        Ok(())
    }

    /// Abbreviate path by replacing home directory with ~
    fn abbreviate_path(path: &PathBuf) -> String {
        if let Some(home_dir) = dirs::home_dir() {
            if let Ok(suffix) = path.strip_prefix(&home_dir) {
                return format!("~/{}", suffix.display());
            }
        }
        path.display().to_string()
    }

    /// Compute display names for all directories
    /// Shows just the base name for unique names, or "base (~/path)" for duplicates
    fn compute_display_names(&mut self) {
        // Group directories by their base name
        let mut base_name_groups: HashMap<String, Vec<usize>> = HashMap::new();

        for (idx, dir) in self.directories.iter().enumerate() {
            let base_name = dir.path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            base_name_groups
                .entry(base_name)
                .or_insert_with(Vec::new)
                .push(idx);
        }

        // Set display names based on uniqueness
        for (base_name, indices) in base_name_groups {
            if indices.len() == 1 {
                // Unique name - just show base name
                let idx = indices[0];
                self.directories[idx].display_name = base_name;
            } else {
                // Duplicate names - show base name with abbreviated path
                for idx in indices {
                    let abbreviated = Self::abbreviate_path(&self.directories[idx].path);
                    self.directories[idx].display_name = format!("{} ({})", base_name, abbreviated);
                }
            }
        }
    }
}

// Snapshot-based cache for BdClient results
#[derive(Clone)]
struct SnapshotCache {
    get_issue_cache: HashMap<String, Issue>,
    // Map from issue_id -> (source_directory, db_path)
    issue_sources: HashMap<String, (String, Option<PathBuf>)>,
}

impl SnapshotCache {
    fn new() -> Self {
        Self {
            get_issue_cache: HashMap::new(),
            issue_sources: HashMap::new(),
        }
    }

    fn clear(&mut self) {
        self.get_issue_cache.clear();
        self.issue_sources.clear();
    }

    fn register_issue_source(&mut self, issue_id: &str, source_directory: &str, db_path: Option<PathBuf>) {
        self.issue_sources.insert(
            issue_id.to_string(),
            (source_directory.to_string(), db_path)
        );
    }

    fn get_issue(&mut self, id: &str) -> Result<Issue, String> {
        // Check cache first
        if let Some(cached_issue) = self.get_issue_cache.get(id) {
            return Ok(cached_issue.clone());
        }

        // Cache miss - fetch from CLI using the registered source
        let db_path = self.issue_sources.get(id).and_then(|(_, path)| path.clone());
        let issue = BdClient::get_issue_uncached(id, db_path.as_ref())?;

        // Store in cache
        self.get_issue_cache.insert(id.to_string(), issue.clone());

        Ok(issue)
    }
}

struct BdClient;

impl BdClient {
    fn list_issues(db_path: Option<&PathBuf>, source_directory: &str) -> Result<Vec<Issue>, String> {
        let mut cmd = Command::new("bd");
        cmd.arg("list").arg("--json");

        // Add --db flag if db_path is provided
        if let Some(path) = db_path {
            // Construct path to .beads/*.db file
            let mut db_file = path.clone();
            db_file.push(".beads");

            // Find the .db file in .beads directory
            if let Ok(entries) = fs::read_dir(&db_file) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if entry_path.extension().and_then(|s| s.to_str()) == Some("db") {
                        cmd.arg("--db").arg(&entry_path);
                        break;
                    }
                }
            }
        }

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to execute bd: {}", e))?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }

        let json = String::from_utf8_lossy(&output.stdout);
        let mut issues: Vec<Issue> = serde_json::from_str(&json)
            .map_err(|e| format!("Failed to parse JSON: {}", e))?;

        // Set source_directory on all issues
        for issue in &mut issues {
            issue.source_directory = source_directory.to_string();
        }

        Ok(issues)
    }

    fn list_issues_from_all(directories: &[DirectoryConfig]) -> Vec<Issue> {
        let mut all_issues = Vec::new();

        for dir_config in directories {
            if !dir_config.visible {
                continue;
            }

            // Use display_name as source_directory identifier
            let source_name = if dir_config.display_name.is_empty() {
                dir_config.path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string()
            } else {
                dir_config.display_name.clone()
            };

            match Self::list_issues(Some(&dir_config.path), &source_name) {
                Ok(mut issues) => {
                    all_issues.append(&mut issues);
                }
                Err(_) => {
                    // Silently skip directories that fail to load
                    // Could add error tracking here if needed
                }
            }
        }

        all_issues
    }

    fn get_issue_uncached(id: &str, db_path: Option<&PathBuf>) -> Result<Issue, String> {
        let mut cmd = Command::new("bd");
        cmd.arg("show").arg(id).arg("--json");

        // Add --db flag if db_path is provided
        if let Some(path) = db_path {
            // Construct path to .beads/*.db file
            let mut db_file = path.clone();
            db_file.push(".beads");

            // Find the .db file in .beads directory
            if let Ok(entries) = fs::read_dir(&db_file) {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if entry_path.extension().and_then(|s| s.to_str()) == Some("db") {
                        cmd.arg("--db").arg(&entry_path);
                        break;
                    }
                }
            }
        }

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to execute bd: {}", e))?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }

        let json = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse JSON: {}", e))
    }

    fn update_issue(id: &str, field: &str, value: &str) -> Result<(), String> {
        let output = Command::new("bd")
            .arg("update")
            .arg(id)
            .arg(format!("--{}", field))
            .arg(value)
            .output()
            .map_err(|e| format!("Failed to execute bd: {}", e))?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct ColumnFilter {
    // Values that are explicitly excluded
    excluded_values: HashSet<String>,
}

impl ColumnFilter {
    fn new() -> Self {
        Self {
            excluded_values: HashSet::new(),
        }
    }

    fn new_with_excluded(excluded: Vec<String>) -> Self {
        Self {
            excluded_values: excluded.into_iter().collect(),
        }
    }

    fn is_filtered(&self, value: &str) -> bool {
        self.excluded_values.contains(value)
    }

    fn toggle_exclude(&mut self, value: String) {
        if self.excluded_values.contains(&value) {
            self.excluded_values.remove(&value);
        } else {
            self.excluded_values.insert(value);
        }
    }

    fn has_active_filters(&self) -> bool {
        !self.excluded_values.is_empty()
    }
}

struct BeadUiApp {
    issues: Vec<Issue>,
    selected_index: Option<usize>,
    filter_text: String,
    error_message: Option<String>,
    sort_by: SortColumn,
    sort_ascending: bool,
    current_issue: Option<Issue>,
    edit_modified: bool,
    hovered_row: Option<usize>,
    split_ratio: f32,  // Ratio of list height to total height (0.0 to 1.0)
    column_filters: HashMap<SortColumn, ColumnFilter>,
    // Map from issue_id -> list of issue_ids that depend on it
    dependents_map: HashMap<String, Vec<String>>,
    // Snapshot-based cache for BdClient calls
    snapshot_cache: SnapshotCache,
    // Application configuration
    config: AppConfig,
}

// Struct to hold pre-computed display values for an issue
struct IssueDisplay {
    original_idx: usize,
    issue: Issue,
    readiness: String,
    blockers_count: usize,
    dependents_count: usize,
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
enum SortColumn {
    Id,
    Directory,
    Title,
    Status,
    Priority,
    Type,
    Assignee,
    Blockers,
    Dependents,
}

impl Default for BeadUiApp {
    fn default() -> Self {
        // Initialize column filters with status excluding "closed" by default
        let mut column_filters = HashMap::new();
        column_filters.insert(
            SortColumn::Status,
            ColumnFilter::new_with_excluded(vec!["closed".to_string()]),
        );

        // Load config from file
        let mut config = AppConfig::load();

        // Auto-add current working directory if not already present
        if let Ok(cwd) = std::env::current_dir() {
            let cwd_exists = config.directories.iter().any(|d| d.path == cwd);

            if !cwd_exists {
                // Add PWD to config as visible by default
                config.directories.push(DirectoryConfig {
                    path: cwd,
                    visible: true,
                    display_name: String::new(), // Will be computed later
                });

                // Compute display names for all directories
                config.compute_display_names();

                // Save the updated config
                let _ = config.save();
            }
        }

        let mut app = Self {
            issues: Vec::new(),
            selected_index: None,
            filter_text: String::new(),
            error_message: None,
            sort_by: SortColumn::Priority,
            sort_ascending: true,
            current_issue: None,
            edit_modified: false,
            hovered_row: None,
            split_ratio: 0.5,  // Start with 50/50 split
            column_filters,
            dependents_map: HashMap::new(),
            snapshot_cache: SnapshotCache::new(),
            config,
        };
        app.refresh();
        app
    }
}

impl BeadUiApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Configure fonts and styles for better system appearance
        Self::setup_custom_fonts(cc);
        Self::default()
    }

    fn load_system_fonts(cc: &eframe::CreationContext<'_>) {
        let mut fonts = egui::FontDefinitions::default();

        // Try to load system UI font
        let system_source = SystemSource::new();

        // Try to find the system UI font based on platform
        let ui_font_result = if cfg!(target_os = "macos") {
            // On macOS, try system UI font (which will be San Francisco on modern macOS)
            system_source.select_best_match(
                &[FamilyName::SansSerif],
                &Properties::new()
            )
        } else if cfg!(target_os = "windows") {
            // On Windows, try Segoe UI
            system_source.select_best_match(
                &[FamilyName::Title("Segoe UI".to_string())],
                &Properties::new()
            ).or_else(|_| {
                system_source.select_best_match(
                    &[FamilyName::SansSerif],
                    &Properties::new()
                )
            })
        } else {
            // On Linux, try common UI fonts
            system_source.select_best_match(
                &[FamilyName::Title("Ubuntu".to_string())],
                &Properties::new()
            ).or_else(|_| {
                system_source.select_best_match(
                    &[FamilyName::Title("Cantarell".to_string())],
                    &Properties::new()
                )
            }).or_else(|_| {
                system_source.select_best_match(
                    &[FamilyName::SansSerif],
                    &Properties::new()
                )
            })
        };

        // Load the system font if found
        if let Ok(handle) = ui_font_result {
            if let Ok(font) = handle.load() {
                if let Some(font_data) = font.copy_font_data() {
                    fonts.font_data.insert(
                        "system_ui".to_owned(),
                        egui::FontData::from_owned(font_data.to_vec()),
                    );

                    // Set system UI font as the first proportional font
                    fonts
                        .families
                        .entry(egui::FontFamily::Proportional)
                        .or_default()
                        .insert(0, "system_ui".to_owned());
                }
            }
        }

        // Load system monospace font
        let mono_font_result = system_source.select_best_match(
            &[FamilyName::Monospace],
            &Properties::new()
        );

        if let Ok(handle) = mono_font_result {
            if let Ok(font) = handle.load() {
                if let Some(font_data) = font.copy_font_data() {
                    fonts.font_data.insert(
                        "system_mono".to_owned(),
                        egui::FontData::from_owned(font_data.to_vec()),
                    );

                    fonts
                        .families
                        .entry(egui::FontFamily::Monospace)
                        .or_default()
                        .insert(0, "system_mono".to_owned());
                }
            }
        }

        cc.egui_ctx.set_fonts(fonts);
    }

    fn setup_custom_fonts(cc: &eframe::CreationContext<'_>) {
        // Load system fonts first
        Self::load_system_fonts(cc);

        // Set up better font sizing that matches system UI conventions
        let mut style = (*cc.egui_ctx.style()).clone();

        // Configure text styles with appropriate sizes for a native look
        // These sizes work well on macOS and other platforms
        style.text_styles = [
            (egui::TextStyle::Small, egui::FontId::new(11.0, egui::FontFamily::Proportional)),
            (egui::TextStyle::Body, egui::FontId::new(13.0, egui::FontFamily::Proportional)),
            (egui::TextStyle::Button, egui::FontId::new(13.0, egui::FontFamily::Proportional)),
            (egui::TextStyle::Heading, egui::FontId::new(17.0, egui::FontFamily::Proportional)),
            (egui::TextStyle::Monospace, egui::FontId::new(12.0, egui::FontFamily::Monospace)),
        ]
        .into();

        // Adjust spacing for a cleaner look
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.window_margin = egui::Margin::same(8.0);

        cc.egui_ctx.set_style(style);
    }

    fn compute_dependents_map(&mut self) {
        // Build a map of issue_id -> list of issues that depend on it
        let mut dependents_map: HashMap<String, Vec<String>> = HashMap::new();

        // We need to load full issue details to get dependencies
        for issue in &self.issues {
            if let Ok(full_issue) = self.snapshot_cache.get_issue(&issue.id) {
                // For each dependency (blocker), add this issue as a dependent
                for dep in &full_issue.dependencies {
                    dependents_map
                        .entry(dep.id.clone())
                        .or_insert_with(Vec::new)
                        .push(issue.id.clone());
                }
            }
        }

        self.dependents_map = dependents_map;
    }

    fn refresh(&mut self) {
        // Clear the snapshot cache on refresh
        self.snapshot_cache.clear();

        // Load issues from all visible directories
        self.issues = BdClient::list_issues_from_all(&self.config.directories);

        // Register all issue sources in the cache
        for dir_config in &self.config.directories {
            if dir_config.visible {
                for issue in &self.issues {
                    if issue.source_directory == dir_config.display_name
                        || (dir_config.display_name.is_empty() && issue.source_directory == dir_config.path.file_name().and_then(|n| n.to_str()).unwrap_or("")) {
                        self.snapshot_cache.register_issue_source(
                            &issue.id,
                            &issue.source_directory,
                            Some(dir_config.path.clone())
                        );
                    }
                }
            }
        }

        self.compute_dependents_map();
        self.error_message = None;
    }

    fn get_blockers_count(&mut self, issue_id: &str) -> usize {
        // Get full issue to count active blockers (dependencies that are not closed)
        if let Ok(full_issue) = self.snapshot_cache.get_issue(issue_id) {
            full_issue.dependencies.iter()
                .filter(|dep| dep.status != "closed")
                .count()
        } else {
            0
        }
    }

    fn get_dependents_count(&self, issue_id: &str) -> usize {
        self.dependents_map.get(issue_id).map(|v| v.len()).unwrap_or(0)
    }

    fn get_readiness(&mut self, issue: &Issue) -> String {
        // Compute readiness based on status and blockers
        match issue.status.as_str() {
            "closed" => "closed".to_string(),
            "in_progress" => "in_progress".to_string(),
            _ => {
                // For open issues, check if they're blocked
                let blockers_count = self.get_blockers_count(&issue.id);
                if blockers_count > 0 {
                    "blocked".to_string()
                } else {
                    "ready".to_string()
                }
            }
        }
    }

    fn get_column_value(&mut self, issue: &Issue, column: SortColumn) -> String {
        match column {
            SortColumn::Id => issue.id.clone(),
            SortColumn::Directory => issue.source_directory.clone(),
            SortColumn::Title => issue.title.clone(),
            SortColumn::Status => self.get_readiness(issue),
            SortColumn::Priority => format!("P{}", issue.priority),
            SortColumn::Type => issue.issue_type.clone(),
            SortColumn::Assignee => issue.assignee.clone().unwrap_or_else(|| "-".to_string()),
            SortColumn::Blockers => self.get_blockers_count(&issue.id).to_string(),
            SortColumn::Dependents => self.get_dependents_count(&issue.id).to_string(),
        }
    }

    fn get_column_cardinality(&mut self, column: SortColumn) -> usize {
        let mut unique_values = HashSet::new();
        for issue in &self.issues.clone() {
            unique_values.insert(self.get_column_value(issue, column));
        }
        unique_values.len()
    }

    fn filtered_and_sorted_issues(&mut self) -> Vec<IssueDisplay> {
        let filter = self.filter_text.to_lowercase();

        // Clone issues before iterating to avoid borrow checker issues
        let issues_clone = self.issues.clone();

        // Pre-compute values that require cache access and clone issues
        let mut filtered: Vec<IssueDisplay> = issues_clone
            .iter()
            .enumerate()
            .filter_map(|(idx, issue)| {
                // Pre-compute values needed for filtering and sorting
                let readiness = self.get_readiness(issue);
                let blockers_count = self.get_blockers_count(&issue.id);
                let dependents_count = self.get_dependents_count(&issue.id);

                // Apply text search filter - search through all visible fields including computed ones
                if !filter.is_empty() {
                    let text_match = issue.id.to_lowercase().contains(&filter)
                        || issue.title.to_lowercase().contains(&filter)
                        || issue.description.to_lowercase().contains(&filter)
                        || issue.status.to_lowercase().contains(&filter)
                        || issue.issue_type.to_lowercase().contains(&filter)
                        || issue
                            .assignee
                            .as_ref()
                            .map(|a| a.to_lowercase().contains(&filter))
                            .unwrap_or(false)
                        || readiness.to_lowercase().contains(&filter)
                        || blockers_count.to_string().contains(&filter)
                        || dependents_count.to_string().contains(&filter);
                    if !text_match {
                        return None;
                    }
                }

                // Apply column filters
                for (column, column_filter) in &self.column_filters {
                    let value = match column {
                        SortColumn::Id => issue.id.clone(),
                        SortColumn::Directory => issue.source_directory.clone(),
                        SortColumn::Title => issue.title.clone(),
                        SortColumn::Status => readiness.clone(),
                        SortColumn::Priority => format!("P{}", issue.priority),
                        SortColumn::Type => issue.issue_type.clone(),
                        SortColumn::Assignee => issue.assignee.clone().unwrap_or_else(|| "-".to_string()),
                        SortColumn::Blockers => blockers_count.to_string(),
                        SortColumn::Dependents => dependents_count.to_string(),
                    };
                    if column_filter.is_filtered(&value) {
                        return None;
                    }
                }

                Some(IssueDisplay {
                    original_idx: idx,
                    issue: issue.clone(),
                    readiness,
                    blockers_count,
                    dependents_count,
                })
            })
            .collect();

        filtered.sort_by(|a, b| {
            let cmp = match self.sort_by {
                SortColumn::Id => a.issue.id.cmp(&b.issue.id),
                SortColumn::Directory => a.issue.source_directory.cmp(&b.issue.source_directory),
                SortColumn::Title => a.issue.title.cmp(&b.issue.title),
                SortColumn::Status => a.readiness.cmp(&b.readiness),
                SortColumn::Priority => a.issue.priority.cmp(&b.issue.priority),
                SortColumn::Type => a.issue.issue_type.cmp(&b.issue.issue_type),
                SortColumn::Assignee => a
                    .issue
                    .assignee
                    .as_ref()
                    .unwrap_or(&String::new())
                    .cmp(b.issue.assignee.as_ref().unwrap_or(&String::new())),
                SortColumn::Blockers => a.blockers_count.cmp(&b.blockers_count),
                SortColumn::Dependents => a.dependents_count.cmp(&b.dependents_count),
            };
            if self.sort_ascending {
                cmp
            } else {
                cmp.reverse()
            }
        });

        filtered
    }

    fn show_sidebar(&mut self, ctx: &egui::Context) {
        let mut config_changed = false;

        egui::SidePanel::left("directories_sidebar")
            .resizable(true)
            .default_width(200.0)
            .show_animated(ctx, !self.config.sidebar_collapsed, |ui| {
                ui.heading("Directories");
                ui.separator();

                // Show list of directories with checkboxes
                for dir in &mut self.config.directories {
                    let mut visible = dir.visible;
                    if ui.checkbox(&mut visible, &dir.display_name).changed() {
                        dir.visible = visible;
                        config_changed = true;
                    }
                }

                ui.separator();

                // Collapse button at bottom
                if ui.button("◀ Collapse").clicked() {
                    self.config.sidebar_collapsed = true;
                    config_changed = true;
                }
            });

        // Show expand button when collapsed
        if self.config.sidebar_collapsed {
            egui::Window::new("expand_sidebar")
                .title_bar(false)
                .resizable(false)
                .fixed_pos([0.0, 100.0])
                .show(ctx, |ui| {
                    if ui.button("▶").clicked() {
                        self.config.sidebar_collapsed = false;
                        config_changed = true;
                    }
                });
        }

        // Save config if anything changed
        if config_changed {
            let _ = self.config.save();
            // Refresh to reload issues with new visibility settings
            self.refresh();
        }
    }

    fn show_list_view(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Show sidebar first (so it's on the left)
        self.show_sidebar(ctx);

        // Header panel
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Beads Issue Tracker").strong());
                ui.separator();
                if ui.button("Refresh").clicked() {
                    self.refresh();
                }

                // Add filter on the right side of the same line
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.text_edit_singleline(&mut self.filter_text);
                    ui.label("Filter:");
                });
            });

            if let Some(ref error) = self.error_message {
                ui.colored_label(egui::Color32::RED, error);
            }
        });

        let mut new_sort_by = None;
        let mut new_selected = None;
        let mut new_hovered_row = None;
        let mut filter_toggle: Option<(SortColumn, String)> = None;

        // Use CentralPanel for the resizable split view
        egui::CentralPanel::default().show(ctx, |ui| {
            let available_height = ui.available_height();

            // Only show split if an issue is selected
            if self.selected_index.is_some() {
                // Calculate list height based on split ratio (min 150px, max available - 150px)
                let min_panel_height = 150.0;
                let list_height = (available_height * self.split_ratio)
                    .max(min_panel_height)
                    .min(available_height - min_panel_height);

                // List panel
                let list_rect = egui::Rect::from_min_size(
                    ui.cursor().min,
                    egui::vec2(ui.available_width(), list_height)
                );
                let mut list_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(list_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT))
                );

                self.show_list_table(&mut list_ui, &mut new_sort_by, &mut new_selected, &mut new_hovered_row, &mut filter_toggle);

                // Separator/divider (draggable)
                let separator_height = 12.0;
                let separator_rect = egui::Rect::from_min_size(
                    egui::pos2(list_rect.min.x, list_rect.max.y),
                    egui::vec2(ui.available_width(), separator_height)
                );

                let separator_id = ui.id().with("split_separator");
                let separator_response = ui.interact(separator_rect, separator_id, egui::Sense::drag());

                // Draw separator with vertical padding
                let separator_color = if separator_response.hovered() || separator_response.dragged() {
                    ui.visuals().widgets.active.bg_fill
                } else {
                    ui.visuals().widgets.inactive.bg_fill
                };
                let top_padding = 2.0;
                let _bottom_padding = 6.0;
                let visual_height = 3.0; // Thin visible line

                let visual_rect = egui::Rect::from_min_size(
                    egui::pos2(separator_rect.min.x, separator_rect.min.y + top_padding),
                    egui::vec2(separator_rect.width(), visual_height)
                );
                ui.painter().rect_filled(visual_rect, 0.0, separator_color);

                // Change cursor on hover
                if separator_response.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                }

                // Handle dragging
                if separator_response.dragged() {
                    if let Some(pointer_pos) = ui.ctx().pointer_latest_pos() {
                        let new_list_height = pointer_pos.y - list_rect.min.y;
                        self.split_ratio = (new_list_height / available_height)
                            .max(min_panel_height / available_height)
                            .min((available_height - min_panel_height) / available_height);
                    }
                }

                // Detail panel
                let detail_rect = egui::Rect::from_min_size(
                    egui::pos2(list_rect.min.x, separator_rect.max.y),
                    egui::vec2(ui.available_width(), available_height - list_height - separator_height)
                );
                let mut detail_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(detail_rect)
                        .layout(egui::Layout::top_down(egui::Align::LEFT))
                );

                if let Some(idx) = self.selected_index {
                    if let Some(issue) = self.issues.get(idx) {
                        let issue_id = issue.id.clone();
                        self.show_detail_view_split(ctx, &mut detail_ui, &issue_id);
                    }
                }
            } else {
                // No issue selected - show list only
                self.show_list_table(ui, &mut new_sort_by, &mut new_selected, &mut new_hovered_row, &mut filter_toggle);
            }
        });

        // Apply changes after borrowing ends
        if let Some(sort_col) = new_sort_by {
            if self.sort_by == sort_col {
                self.sort_ascending = !self.sort_ascending;
            } else {
                self.sort_by = sort_col;
                self.sort_ascending = true;
            }
        }

        if let Some(selected) = new_selected {
            self.selected_index = selected;
        }

        if let Some(hovered) = new_hovered_row {
            self.hovered_row = hovered;
        } else {
            self.hovered_row = None;
        }

        // Apply filter toggle if requested
        if let Some((column, value)) = filter_toggle {
            self.column_filters
                .entry(column)
                .or_insert_with(ColumnFilter::new)
                .toggle_exclude(value);
        }

        // Keyboard navigation
        ctx.input(|i| {
            if i.key_pressed(egui::Key::ArrowDown) {
                if let Some(idx) = self.selected_index {
                    if idx + 1 < self.issues.len() {
                        self.selected_index = Some(idx + 1);
                    }
                } else if !self.issues.is_empty() {
                    self.selected_index = Some(0);
                }
            }

            if i.key_pressed(egui::Key::ArrowUp) {
                if let Some(idx) = self.selected_index {
                    if idx > 0 {
                        self.selected_index = Some(idx - 1);
                    }
                }
            }
        });
    }

    fn show_list_table(
        &mut self,
        ui: &mut egui::Ui,
        new_sort_by: &mut Option<SortColumn>,
        new_selected: &mut Option<Option<usize>>,
        new_hovered_row: &mut Option<Option<usize>>,
        filter_toggle: &mut Option<(SortColumn, String)>,
    ) {
        let filtered = self.filtered_and_sorted_issues();

        // Pre-compute cardinalities to avoid borrow checker issues in context menus
        let id_cardinality = self.get_column_cardinality(SortColumn::Id);
        let directory_cardinality = self.get_column_cardinality(SortColumn::Directory);
        let title_cardinality = self.get_column_cardinality(SortColumn::Title);
        let status_cardinality = self.get_column_cardinality(SortColumn::Status);
        let priority_cardinality = self.get_column_cardinality(SortColumn::Priority);
        let type_cardinality = self.get_column_cardinality(SortColumn::Type);
        let assignee_cardinality = self.get_column_cardinality(SortColumn::Assignee);
        let blockers_cardinality = self.get_column_cardinality(SortColumn::Blockers);
        let dependents_cardinality = self.get_column_cardinality(SortColumn::Dependents);

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(100.0).resizable(true))  // ID
            .column(Column::initial(120.0).resizable(true))  // Directory
            .column(Column::remainder().resizable(true))      // Title
            .column(Column::initial(100.0).resizable(true))  // Status
            .column(Column::initial(70.0).resizable(true))   // Priority
            .column(Column::initial(100.0).resizable(true))  // Type
            .column(Column::initial(120.0).resizable(true))  // Assignee
            .column(Column::initial(80.0).resizable(true))   // Blockers
            .column(Column::initial(80.0).resizable(true))   // Dependents
            .header(25.0, |mut header| {
                header.col(|ui| {
                    if self.sortable_header_ui(ui, "ID", SortColumn::Id, id_cardinality, filter_toggle) {
                        *new_sort_by = Some(SortColumn::Id);
                    }
                });
                header.col(|ui| {
                    if self.sortable_header_ui(ui, "Directory", SortColumn::Directory, directory_cardinality, filter_toggle) {
                        *new_sort_by = Some(SortColumn::Directory);
                    }
                });
                header.col(|ui| {
                    if self.sortable_header_ui(ui, "Title", SortColumn::Title, title_cardinality, filter_toggle) {
                        *new_sort_by = Some(SortColumn::Title);
                    }
                });
                header.col(|ui| {
                    if self.sortable_header_ui(ui, "Status", SortColumn::Status, status_cardinality, filter_toggle) {
                        *new_sort_by = Some(SortColumn::Status);
                    }
                });
                header.col(|ui| {
                    if self.sortable_header_ui(ui, "Priority", SortColumn::Priority, priority_cardinality, filter_toggle) {
                        *new_sort_by = Some(SortColumn::Priority);
                    }
                });
                header.col(|ui| {
                    if self.sortable_header_ui(ui, "Type", SortColumn::Type, type_cardinality, filter_toggle) {
                        *new_sort_by = Some(SortColumn::Type);
                    }
                });
                header.col(|ui| {
                    if self.sortable_header_ui(ui, "Assignee", SortColumn::Assignee, assignee_cardinality, filter_toggle) {
                        *new_sort_by = Some(SortColumn::Assignee);
                    }
                });
                header.col(|ui| {
                    if self.sortable_header_ui(ui, "Blockers", SortColumn::Blockers, blockers_cardinality, filter_toggle) {
                        *new_sort_by = Some(SortColumn::Blockers);
                    }
                });
                header.col(|ui| {
                    if self.sortable_header_ui(ui, "Dependents", SortColumn::Dependents, dependents_cardinality, filter_toggle) {
                        *new_sort_by = Some(SortColumn::Dependents);
                    }
                });
            })
            .body(|body| {
                body.rows(20.0, filtered.len(), |mut row| {
                    let row_index = row.index();
                    if let Some(display) = filtered.get(row_index) {
                        let original_idx = display.original_idx;
                        let issue = &display.issue;
                        let is_selected = self.selected_index == Some(original_idx);
                        let is_row_hovered = self.hovered_row == Some(original_idx);

                        row.set_selected(is_selected);

                        let mut any_cell_hovered = false;

                        row.col(|ui| {
                            let available_size = ui.available_size();
                            let (id, rect) = ui.allocate_space(available_size);
                            let response = ui.interact(rect, id, egui::Sense::click());

                            if response.hovered() {
                                any_cell_hovered = true;
                            }

                            if is_row_hovered {
                                ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
                            }

                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                            );
                            child_ui.add(egui::Label::new(&issue.id).selectable(false));

                            if response.clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                            // No context menu for ID column (not useful for filtering)
                        });

                        // Directory column
                        row.col(|ui| {
                            let available_size = ui.available_size();
                            let (id, rect) = ui.allocate_space(available_size);
                            let response = ui.interact(rect, id, egui::Sense::click());

                            if response.hovered() {
                                any_cell_hovered = true;
                            }

                            if is_row_hovered {
                                ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
                            }

                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                            );
                            child_ui.add(egui::Label::new(&issue.source_directory).selectable(false));

                            if response.clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(original_idx));
                            }

                            let directory_value = issue.source_directory.clone();
                            response.context_menu(|ui| {
                                if directory_cardinality > 20 {
                                    ui.label(format!("⚠ High cardinality ({} values)", directory_cardinality));
                                    ui.label("Filtering not available");
                                } else {
                                    let current_filter = self.column_filters.get(&SortColumn::Directory);
                                    let is_filtered = current_filter
                                        .map(|f| f.is_filtered(&directory_value))
                                        .unwrap_or(false);

                                    if ui.button(if is_filtered {
                                        format!("✓ Include \"{}\"", directory_value)
                                    } else {
                                        format!("✗ Exclude \"{}\"", directory_value)
                                    }).clicked() {
                                        *filter_toggle = Some((SortColumn::Directory, directory_value.clone()));
                                        ui.close_menu();
                                    }
                                }
                            });
                        });

                        row.col(|ui| {
                            let available_size = ui.available_size();
                            let (id, rect) = ui.allocate_space(available_size);
                            let response = ui.interact(rect, id, egui::Sense::click());

                            if response.hovered() {
                                any_cell_hovered = true;
                            }

                            if is_row_hovered {
                                ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
                            }

                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                            );
                            child_ui.add(egui::Label::new(&issue.title).selectable(false));

                            if response.clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                            // No context menu for Title column (not useful for filtering)
                        });

                        row.col(|ui| {
                            let available_size = ui.available_size();
                            let (id, rect) = ui.allocate_space(available_size);
                            let response = ui.interact(rect, id, egui::Sense::click());

                            if response.hovered() {
                                any_cell_hovered = true;
                            }

                            if is_row_hovered {
                                ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
                            }

                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                            );
                            let status_text = &display.readiness;
                            child_ui.add(egui::Label::new(status_text).selectable(false));

                            if response.clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(original_idx));
                            }

                            let status_value = status_text.clone();
                            response.context_menu(|ui| {
                                if status_cardinality > 20 {
                                    ui.label(format!("⚠ High cardinality ({} values)", status_cardinality));
                                    ui.label("Filtering not available");
                                } else {
                                    let current_filter = self.column_filters.get(&SortColumn::Status);
                                    let is_filtered = current_filter
                                        .map(|f| f.is_filtered(&status_value))
                                        .unwrap_or(false);

                                    if ui.button(if is_filtered {
                                        format!("✓ Include \"{}\"", status_value)
                                    } else {
                                        format!("✗ Exclude \"{}\"", status_value)
                                    }).clicked() {
                                        *filter_toggle = Some((SortColumn::Status, status_value.clone()));
                                        ui.close_menu();
                                    }
                                }
                            });
                        });

                        row.col(|ui| {
                            let available_size = ui.available_size();
                            let (id, rect) = ui.allocate_space(available_size);
                            let response = ui.interact(rect, id, egui::Sense::click());

                            if response.hovered() {
                                any_cell_hovered = true;
                            }

                            if is_row_hovered {
                                ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
                            }

                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                            );
                            let priority_text = format!("P{}", issue.priority);
                            child_ui.add(egui::Label::new(&priority_text).selectable(false));

                            if response.clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(original_idx));
                            }

                            let priority_value = priority_text.clone();
                            response.context_menu(|ui| {
                                if priority_cardinality > 20 {
                                    ui.label(format!("⚠ High cardinality ({} values)", priority_cardinality));
                                    ui.label("Filtering not available");
                                } else {
                                    let current_filter = self.column_filters.get(&SortColumn::Priority);
                                    let is_filtered = current_filter
                                        .map(|f| f.is_filtered(&priority_value))
                                        .unwrap_or(false);

                                    if ui.button(if is_filtered {
                                        format!("✓ Include \"{}\"", priority_value)
                                    } else {
                                        format!("✗ Exclude \"{}\"", priority_value)
                                    }).clicked() {
                                        *filter_toggle = Some((SortColumn::Priority, priority_value.clone()));
                                        ui.close_menu();
                                    }
                                }
                            });
                        });

                        row.col(|ui| {
                            let available_size = ui.available_size();
                            let (id, rect) = ui.allocate_space(available_size);
                            let response = ui.interact(rect, id, egui::Sense::click());

                            if response.hovered() {
                                any_cell_hovered = true;
                            }

                            if is_row_hovered {
                                ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
                            }

                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                            );
                            child_ui.add(egui::Label::new(&issue.issue_type).selectable(false));

                            if response.clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(original_idx));
                            }

                            let type_value = issue.issue_type.clone();
                            response.context_menu(|ui| {
                                if type_cardinality > 20 {
                                    ui.label(format!("⚠ High cardinality ({} values)", type_cardinality));
                                    ui.label("Filtering not available");
                                } else {
                                    let current_filter = self.column_filters.get(&SortColumn::Type);
                                    let is_filtered = current_filter
                                        .map(|f| f.is_filtered(&type_value))
                                        .unwrap_or(false);

                                    if ui.button(if is_filtered {
                                        format!("✓ Include \"{}\"", type_value)
                                    } else {
                                        format!("✗ Exclude \"{}\"", type_value)
                                    }).clicked() {
                                        *filter_toggle = Some((SortColumn::Type, type_value.clone()));
                                        ui.close_menu();
                                    }
                                }
                            });
                        });

                        row.col(|ui| {
                            let available_size = ui.available_size();
                            let (id, rect) = ui.allocate_space(available_size);
                            let response = ui.interact(rect, id, egui::Sense::click());

                            if response.hovered() {
                                any_cell_hovered = true;
                            }

                            if is_row_hovered {
                                ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
                            }

                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                            );
                            let assignee_text = issue.assignee.as_ref().unwrap_or(&"-".to_string()).clone();
                            child_ui.add(egui::Label::new(&assignee_text).selectable(false));

                            if response.clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(original_idx));
                            }

                            let assignee_value = assignee_text.clone();
                            response.context_menu(|ui| {
                                if assignee_cardinality > 20 {
                                    ui.label(format!("⚠ High cardinality ({} values)", assignee_cardinality));
                                    ui.label("Filtering not available");
                                } else {
                                    let current_filter = self.column_filters.get(&SortColumn::Assignee);
                                    let is_filtered = current_filter
                                        .map(|f| f.is_filtered(&assignee_value))
                                        .unwrap_or(false);

                                    if ui.button(if is_filtered {
                                        format!("✓ Include \"{}\"", assignee_value)
                                    } else {
                                        format!("✗ Exclude \"{}\"", assignee_value)
                                    }).clicked() {
                                        *filter_toggle = Some((SortColumn::Assignee, assignee_value.clone()));
                                        ui.close_menu();
                                    }
                                }
                            });
                        });

                        // Blockers column
                        row.col(|ui| {
                            let available_size = ui.available_size();
                            let (id, rect) = ui.allocate_space(available_size);
                            let response = ui.interact(rect, id, egui::Sense::click());

                            if response.hovered() {
                                any_cell_hovered = true;
                            }

                            if is_row_hovered {
                                ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
                            }

                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                            );
                            let blockers_count = display.blockers_count;
                            child_ui.add(egui::Label::new(blockers_count.to_string()).selectable(false));

                            if response.clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                        });

                        // Dependents column
                        row.col(|ui| {
                            let available_size = ui.available_size();
                            let (id, rect) = ui.allocate_space(available_size);
                            let response = ui.interact(rect, id, egui::Sense::click());

                            if response.hovered() {
                                any_cell_hovered = true;
                            }

                            if is_row_hovered {
                                ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.hovered.bg_fill);
                            }

                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center))
                            );
                            let dependents_count = display.dependents_count;
                            child_ui.add(egui::Label::new(dependents_count.to_string()).selectable(false));

                            if response.clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(original_idx));
                            }
                        });

                        if any_cell_hovered {
                            *new_hovered_row = Some(Some(original_idx));
                        }
                    }
                });
            });
    }

    fn sortable_header_ui(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        column: SortColumn,
        cardinality: usize,
        filter_toggle: &mut Option<(SortColumn, String)>,
    ) -> bool {
        let mut text = label.to_string();

        // Add filter indicator if column has active filters
        if let Some(filter) = self.column_filters.get(&column) {
            if filter.has_active_filters() {
                text = format!("{} •", text);
            }
        }

        // Add sort indicator if this is the sort column
        if self.sort_by == column {
            text = format!("{} {}", text, if self.sort_ascending { "▲" } else { "▼" });
        }

        let button_response = ui.button(text);
        let clicked = button_response.clicked();

        // Skip filter menu for ID and Title columns (always high cardinality)
        let skip_filter_menu = matches!(column, SortColumn::Id | SortColumn::Title);

        // Add context menu to header for filter management
        if !skip_filter_menu {
            // Pre-compute values outside the closure to avoid borrow issues
            let values: Vec<String> = if cardinality <= 20 {
                let issues_clone = self.issues.clone();
                let mut vals: Vec<String> = issues_clone
                    .iter()
                    .map(|issue| self.get_column_value(issue, column))
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                vals.sort();
                vals
            } else {
                Vec::new()
            };

            let current_filter_excluded = self.column_filters.get(&column)
                .map(|f| f.excluded_values.clone())
                .unwrap_or_default();
            let has_active_filters = !current_filter_excluded.is_empty();

            button_response.context_menu(|ui| {
                ui.label(format!("{} Column Filters", label));
                ui.separator();

                if cardinality > 20 {
                    ui.label(format!("⚠ High cardinality ({} values)", cardinality));
                    ui.label("Filtering not available");
                } else {
                    for value in &values {
                        let is_filtered = current_filter_excluded.contains(value);

                        if ui
                            .button(if is_filtered {
                                format!("☐ {}", value)
                            } else {
                                format!("☑ {}", value)
                            })
                            .clicked()
                        {
                            *filter_toggle = Some((column, value.clone()));
                        }
                    }

                    // Add "Clear all filters" option if there are active filters
                    if has_active_filters {
                        ui.separator();
                        if ui.button("Clear all filters").clicked() {
                            // Toggle the first filtered value to clear it
                            if let Some(excluded_value) = current_filter_excluded.iter().next() {
                                *filter_toggle = Some((column, excluded_value.clone()));
                            }
                        }
                    }
                }
            });
        }

        clicked
    }

    fn show_detail_view_split(&mut self, _ctx: &egui::Context, ui: &mut egui::Ui, issue_id: &str) {
        // Load issue if not already loaded or if different issue
        if self.current_issue.is_none() || self.current_issue.as_ref().map(|i| &i.id) != Some(&issue_id.to_string()) {
            match self.snapshot_cache.get_issue(issue_id) {
                Ok(issue) => {
                    self.current_issue = Some(issue);
                    self.edit_modified = false;
                    self.error_message = None;
                }
                Err(e) => {
                    self.error_message = Some(format!("Error loading issue: {}", e));
                    self.current_issue = None;
                }
            }
        }

        let mut should_save = false;
        let mut should_refresh = false;
        let mut nav_to_issue_idx = None;

        // Header
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("Issue: {}", issue_id)).strong());
            ui.separator();

            if ui.button("Refresh").clicked() {
                should_refresh = true;
            }

            ui.separator();

            if self.edit_modified {
                if ui.button("💾 Save").clicked() {
                    should_save = true;
                }
                ui.colored_label(egui::Color32::YELLOW, "Unsaved changes");
            }
        });

        if let Some(ref error) = self.error_message {
            ui.colored_label(egui::Color32::RED, error);
        }

        ui.separator();

        // Content
        egui::ScrollArea::vertical().id_salt("detail_scroll").show(ui, |ui| {
            if let Some(ref mut issue) = self.current_issue {
                ui.horizontal(|ui| {
                    ui.label("ID:");
                    ui.label(&issue.id);
                });

                ui.horizontal(|ui| {
                    ui.label("Title:");
                    let title_edit = egui::TextEdit::singleline(&mut issue.title)
                        .desired_width(f32::INFINITY);
                    if ui.add(title_edit).changed() {
                        self.edit_modified = true;
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Status:");
                    let old_status = issue.status.clone();
                    egui::ComboBox::from_id_salt("status_combo")
                        .selected_text(&issue.status)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut issue.status, "open".to_string(), "open");
                            ui.selectable_value(&mut issue.status, "in_progress".to_string(), "in_progress");
                            ui.selectable_value(&mut issue.status, "closed".to_string(), "closed");
                        });
                    if issue.status != old_status {
                        self.edit_modified = true;
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Priority:");
                    let old_priority = issue.priority;
                    egui::ComboBox::from_id_salt("priority_combo")
                        .selected_text(format!("P{}", issue.priority))
                        .show_ui(ui, |ui| {
                            for p in 0..=4 {
                                ui.selectable_value(&mut issue.priority, p, format!("P{}", p));
                            }
                        });
                    if issue.priority != old_priority {
                        self.edit_modified = true;
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Type:");
                    ui.label(&issue.issue_type);
                });

                ui.horizontal(|ui| {
                    ui.label("Assignee:");
                    let mut assignee_text = issue.assignee.clone().unwrap_or_default();
                    let assignee_edit = egui::TextEdit::singleline(&mut assignee_text)
                        .desired_width(f32::INFINITY);
                    if ui.add(assignee_edit).changed() {
                        issue.assignee = if assignee_text.is_empty() {
                            None
                        } else {
                            Some(assignee_text)
                        };
                        self.edit_modified = true;
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Created:");
                    ui.label(&issue.created_at);
                });

                ui.horizontal(|ui| {
                    ui.label("Updated:");
                    ui.label(&issue.updated_at);
                });

                ui.separator();
                ui.label("Description:");
                ui.label(&issue.description);

                ui.separator();
                ui.label("Notes:");
                let mut notes_text = issue.notes.clone().unwrap_or_default();
                let notes_edit = egui::TextEdit::multiline(&mut notes_text)
                    .desired_width(f32::INFINITY)
                    .id_source("notes_edit");
                let notes_response = ui.add(notes_edit);
                if notes_response.changed() {
                    issue.notes = if notes_text.is_empty() {
                        None
                    } else {
                        Some(notes_text)
                    };
                    self.edit_modified = true;
                    // Request focus to prevent losing it when Save button appears
                    notes_response.request_focus();
                }

                // Always show Blockers section (issues that must be completed before this one)
                ui.separator();
                ui.label("Blockers (issues blocking this one):");
                if issue.dependencies.is_empty() {
                    ui.label("  None");
                } else {
                    for dep in &issue.dependencies {
                        ui.horizontal(|ui| {
                            if ui.button(&dep.id).clicked() {
                                // Find the index of this dependency in the issues list
                                if let Some(dep_idx) = self.issues.iter().position(|i| i.id == dep.id) {
                                    nav_to_issue_idx = Some(dep_idx);
                                }
                            }
                            ui.label(format!("- {}", dep.title));
                        });
                    }
                }

                // Always show Dependents section (issues blocked by this one)
                ui.separator();
                ui.label("Dependents (issues blocked by this one):");
                if let Some(dependent_ids) = self.dependents_map.get(&issue.id) {
                    for dependent_id in dependent_ids {
                        if let Some(dependent) = self.issues.iter().find(|i| &i.id == dependent_id) {
                            ui.horizontal(|ui| {
                                if ui.button(&dependent.id).clicked() {
                                    // Find the index of this dependent in the issues list
                                    if let Some(dep_idx) = self.issues.iter().position(|i| i.id == dependent.id) {
                                        nav_to_issue_idx = Some(dep_idx);
                                    }
                                }
                                ui.label(format!("- {}", dependent.title));
                            });
                        }
                    }
                } else {
                    ui.label("  None");
                }
            }
        });

        // Handle actions after borrowing
        if should_refresh {
            self.current_issue = None;
            self.edit_modified = false;
        }

        if should_save {
            if let Some(issue) = self.current_issue.clone() {
                self.save_issue_changes(&issue);
            }
        }

        if let Some(new_idx) = nav_to_issue_idx {
            self.selected_index = Some(new_idx);
            self.current_issue = None;
            self.edit_modified = false;
        }
    }

    fn save_issue_changes(&mut self, issue: &Issue) {
        let mut errors = Vec::new();

        // Update title
        if let Err(e) = BdClient::update_issue(&issue.id, "title", &issue.title) {
            errors.push(format!("title: {}", e));
        }

        // Update status
        if let Err(e) = BdClient::update_issue(&issue.id, "status", &issue.status) {
            errors.push(format!("status: {}", e));
        }

        // Update priority
        if let Err(e) = BdClient::update_issue(&issue.id, "priority", &issue.priority.to_string()) {
            errors.push(format!("priority: {}", e));
        }

        // Update assignee
        if let Some(ref assignee) = issue.assignee {
            if let Err(e) = BdClient::update_issue(&issue.id, "assignee", assignee) {
                errors.push(format!("assignee: {}", e));
            }
        }

        // Update notes
        if let Some(ref notes) = issue.notes {
            if let Err(e) = BdClient::update_issue(&issue.id, "notes", notes) {
                errors.push(format!("notes: {}", e));
            }
        }

        if errors.is_empty() {
            self.error_message = None;
            self.edit_modified = false;
            // Reload the issue to get fresh data
            self.current_issue = None;
            // Refresh the list
            self.refresh();
        } else {
            self.error_message = Some(format!("Failed to save: {}", errors.join(", ")));
        }
    }
}

impl eframe::App for BeadUiApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.show_list_view(ctx, frame);
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1200.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Beads UI",
        options,
        Box::new(|cc| Ok(Box::new(BeadUiApp::new(cc)))),
    )
}
