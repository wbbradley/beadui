use eframe::egui;
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::process::Command;

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
}

struct BdClient;

impl BdClient {
    fn list_issues() -> Result<Vec<Issue>, String> {
        let output = Command::new("bd")
            .arg("list")
            .arg("--json")
            .output()
            .map_err(|e| format!("Failed to execute bd: {}", e))?;

        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).to_string());
        }

        let json = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&json).map_err(|e| format!("Failed to parse JSON: {}", e))
    }

    fn get_issue(id: &str) -> Result<Issue, String> {
        let output = Command::new("bd")
            .arg("show")
            .arg(id)
            .arg("--json")
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
}

#[derive(PartialEq, Eq, Hash, Clone, Copy, Debug)]
enum SortColumn {
    Id,
    Title,
    Status,
    Priority,
    Type,
    Assignee,
}

impl Default for BeadUiApp {
    fn default() -> Self {
        // Initialize column filters with status excluding "closed" by default
        let mut column_filters = HashMap::new();
        column_filters.insert(
            SortColumn::Status,
            ColumnFilter::new_with_excluded(vec!["closed".to_string()]),
        );

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

    fn setup_custom_fonts(cc: &eframe::CreationContext<'_>) {
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

    fn refresh(&mut self) {
        match BdClient::list_issues() {
            Ok(issues) => {
                self.issues = issues;
                self.error_message = None;
            }
            Err(e) => {
                self.error_message = Some(format!("Failed to load issues: {}", e));
            }
        }
    }

    fn get_column_value(&self, issue: &Issue, column: SortColumn) -> String {
        match column {
            SortColumn::Id => issue.id.clone(),
            SortColumn::Title => issue.title.clone(),
            SortColumn::Status => issue.status.clone(),
            SortColumn::Priority => format!("P{}", issue.priority),
            SortColumn::Type => issue.issue_type.clone(),
            SortColumn::Assignee => issue.assignee.clone().unwrap_or_else(|| "-".to_string()),
        }
    }

    fn get_column_cardinality(&self, column: SortColumn) -> usize {
        let mut unique_values = HashSet::new();
        for issue in &self.issues {
            unique_values.insert(self.get_column_value(issue, column));
        }
        unique_values.len()
    }

    fn filtered_and_sorted_issues(&self) -> Vec<(usize, &Issue)> {
        let filter = self.filter_text.to_lowercase();
        let mut filtered: Vec<(usize, &Issue)> = self
            .issues
            .iter()
            .enumerate()
            .filter(|(_, issue)| {
                // Apply text search filter
                if !filter.is_empty() {
                    let text_match = issue.id.to_lowercase().contains(&filter)
                        || issue.title.to_lowercase().contains(&filter)
                        || issue.description.to_lowercase().contains(&filter)
                        || issue.status.to_lowercase().contains(&filter)
                        || issue
                            .assignee
                            .as_ref()
                            .map(|a| a.to_lowercase().contains(&filter))
                            .unwrap_or(false);
                    if !text_match {
                        return false;
                    }
                }

                // Apply column filters
                for (column, column_filter) in &self.column_filters {
                    let value = self.get_column_value(issue, *column);
                    if column_filter.is_filtered(&value) {
                        return false;
                    }
                }

                true
            })
            .collect();

        filtered.sort_by(|(_, a), (_, b)| {
            let cmp = match self.sort_by {
                SortColumn::Id => a.id.cmp(&b.id),
                SortColumn::Title => a.title.cmp(&b.title),
                SortColumn::Status => a.status.cmp(&b.status),
                SortColumn::Priority => a.priority.cmp(&b.priority),
                SortColumn::Type => a.issue_type.cmp(&b.issue_type),
                SortColumn::Assignee => a
                    .assignee
                    .as_ref()
                    .unwrap_or(&String::new())
                    .cmp(b.assignee.as_ref().unwrap_or(&String::new())),
            };
            if self.sort_ascending {
                cmp
            } else {
                cmp.reverse()
            }
        });

        filtered
    }

    fn show_list_view(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Header panel
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Beads Issue Tracker").strong());
                ui.separator();
                if ui.button("Refresh").clicked() {
                    self.refresh();
                }
            });

            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.text_edit_singleline(&mut self.filter_text);
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
        &self,
        ui: &mut egui::Ui,
        new_sort_by: &mut Option<SortColumn>,
        new_selected: &mut Option<Option<usize>>,
        new_hovered_row: &mut Option<Option<usize>>,
        filter_toggle: &mut Option<(SortColumn, String)>,
    ) {
        let filtered = self.filtered_and_sorted_issues();

        // Pre-compute cardinalities to avoid borrow checker issues in context menus
        let id_cardinality = self.get_column_cardinality(SortColumn::Id);
        let title_cardinality = self.get_column_cardinality(SortColumn::Title);
        let status_cardinality = self.get_column_cardinality(SortColumn::Status);
        let priority_cardinality = self.get_column_cardinality(SortColumn::Priority);
        let type_cardinality = self.get_column_cardinality(SortColumn::Type);
        let assignee_cardinality = self.get_column_cardinality(SortColumn::Assignee);

        TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::initial(100.0).resizable(true))  // ID
            .column(Column::remainder().resizable(true))      // Title
            .column(Column::initial(100.0).resizable(true))  // Status
            .column(Column::initial(70.0).resizable(true))   // Priority
            .column(Column::initial(100.0).resizable(true))  // Type
            .column(Column::initial(120.0).resizable(true))  // Assignee
            .header(25.0, |mut header| {
                header.col(|ui| {
                    if self.sortable_header_ui(ui, "ID", SortColumn::Id, id_cardinality, filter_toggle) {
                        *new_sort_by = Some(SortColumn::Id);
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
            })
            .body(|body| {
                body.rows(20.0, filtered.len(), |mut row| {
                    let row_index = row.index();
                    if let Some((original_idx, issue)) = filtered.get(row_index) {
                        let is_selected = self.selected_index == Some(*original_idx);
                        let is_row_hovered = self.hovered_row == Some(*original_idx);

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
                                *new_selected = Some(Some(*original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(*original_idx));
                            }
                            // No context menu for ID column (not useful for filtering)
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
                                *new_selected = Some(Some(*original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(*original_idx));
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
                            child_ui.add(egui::Label::new(&issue.status).selectable(false));

                            if response.clicked() {
                                *new_selected = Some(Some(*original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(*original_idx));
                            }

                            response.context_menu(|ui| {
                                if status_cardinality > 20 {
                                    ui.label(format!("⚠ High cardinality ({} values)", status_cardinality));
                                    ui.label("Filtering not available");
                                } else {
                                    let current_filter = self.column_filters.get(&SortColumn::Status);
                                    let is_filtered = current_filter
                                        .map(|f| f.is_filtered(&issue.status))
                                        .unwrap_or(false);

                                    if ui.button(if is_filtered {
                                        format!("✓ Include \"{}\"", issue.status)
                                    } else {
                                        format!("✗ Exclude \"{}\"", issue.status)
                                    }).clicked() {
                                        *filter_toggle = Some((SortColumn::Status, issue.status.clone()));
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
                                *new_selected = Some(Some(*original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(*original_idx));
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
                                *new_selected = Some(Some(*original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(*original_idx));
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
                                *new_selected = Some(Some(*original_idx));
                            }
                            if response.double_clicked() {
                                *new_selected = Some(Some(*original_idx));
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

                        if any_cell_hovered {
                            *new_hovered_row = Some(Some(*original_idx));
                        }
                    }
                });
            });
    }

    fn sortable_header_ui(
        &self,
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
                text = format!("{} 🔽", text);
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
            button_response.context_menu(|ui| {
                ui.label(format!("{} Column Filters", label));
                ui.separator();

                if cardinality > 20 {
                    ui.label(format!("⚠ High cardinality ({} values)", cardinality));
                    ui.label("Filtering not available");
                } else {
                // Get all unique values for this column
                let mut values: Vec<String> = self
                    .issues
                    .iter()
                    .map(|issue| self.get_column_value(issue, column))
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                values.sort();

                let current_filter = self.column_filters.get(&column);

                for value in values {
                    let is_filtered = current_filter
                        .map(|f| f.is_filtered(&value))
                        .unwrap_or(false);

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
                if let Some(filter) = current_filter {
                    if filter.has_active_filters() {
                        ui.separator();
                        if ui.button("Clear all filters").clicked() {
                            // Toggle each filtered value to clear them
                            for excluded_value in &filter.excluded_values {
                                *filter_toggle = Some((column, excluded_value.clone()));
                                break; // Only do one at a time, user can click multiple times
                            }
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
            match BdClient::get_issue(issue_id) {
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

                if !issue.dependencies.is_empty() {
                    ui.separator();
                    ui.label("Dependencies (Blocks this issue):");
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
