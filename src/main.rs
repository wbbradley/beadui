use eframe::egui;
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};
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
}

#[derive(PartialEq, Clone, Copy)]
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
        };
        app.refresh();
        app
    }
}

impl BeadUiApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::default()
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

    fn filtered_and_sorted_issues(&self) -> Vec<(usize, &Issue)> {
        let filter = self.filter_text.to_lowercase();
        let mut filtered: Vec<(usize, &Issue)> = self
            .issues
            .iter()
            .enumerate()
            .filter(|(_, issue)| {
                if filter.is_empty() {
                    return true;
                }
                issue.id.to_lowercase().contains(&filter)
                    || issue.title.to_lowercase().contains(&filter)
                    || issue.description.to_lowercase().contains(&filter)
                    || issue.status.to_lowercase().contains(&filter)
                    || issue
                        .assignee
                        .as_ref()
                        .map(|a| a.to_lowercase().contains(&filter))
                        .unwrap_or(false)
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

        // Top half - List View
        egui::TopBottomPanel::top("list_panel").min_height(300.0).show(ctx, |ui| {
            let filtered = self.filtered_and_sorted_issues();

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
                        if self.sortable_header_ui(ui, "ID", SortColumn::Id) {
                            new_sort_by = Some(SortColumn::Id);
                        }
                    });
                    header.col(|ui| {
                        if self.sortable_header_ui(ui, "Title", SortColumn::Title) {
                            new_sort_by = Some(SortColumn::Title);
                        }
                    });
                    header.col(|ui| {
                        if self.sortable_header_ui(ui, "Status", SortColumn::Status) {
                            new_sort_by = Some(SortColumn::Status);
                        }
                    });
                    header.col(|ui| {
                        if self.sortable_header_ui(ui, "Priority", SortColumn::Priority) {
                            new_sort_by = Some(SortColumn::Priority);
                        }
                    });
                    header.col(|ui| {
                        if self.sortable_header_ui(ui, "Type", SortColumn::Type) {
                            new_sort_by = Some(SortColumn::Type);
                        }
                    });
                    header.col(|ui| {
                        if self.sortable_header_ui(ui, "Assignee", SortColumn::Assignee) {
                            new_sort_by = Some(SortColumn::Assignee);
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
                                    new_selected = Some(Some(*original_idx));
                                }
                                if response.double_clicked() {
                                    new_selected = Some(Some(*original_idx));
                                }
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
                                    new_selected = Some(Some(*original_idx));
                                }
                                if response.double_clicked() {
                                    new_selected = Some(Some(*original_idx));
                                }
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
                                    new_selected = Some(Some(*original_idx));
                                }
                                if response.double_clicked() {
                                    new_selected = Some(Some(*original_idx));
                                }
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
                                child_ui.add(egui::Label::new(format!("P{}", issue.priority)).selectable(false));

                                if response.clicked() {
                                    new_selected = Some(Some(*original_idx));
                                }
                                if response.double_clicked() {
                                    new_selected = Some(Some(*original_idx));
                                }
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
                                    new_selected = Some(Some(*original_idx));
                                }
                                if response.double_clicked() {
                                    new_selected = Some(Some(*original_idx));
                                }
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
                                child_ui.add(egui::Label::new(issue.assignee.as_ref().unwrap_or(&"-".to_string())).selectable(false));

                                if response.clicked() {
                                    new_selected = Some(Some(*original_idx));
                                }
                                if response.double_clicked() {
                                    new_selected = Some(Some(*original_idx));
                                }
                            });

                            if any_cell_hovered {
                                new_hovered_row = Some(Some(*original_idx));
                            }
                        }
                    });
                });
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

        if let Some(view) = new_view {
            self.current_view = view;
        }

        if let Some(hovered) = new_hovered_row {
            self.hovered_row = hovered;
        } else {
            self.hovered_row = None;
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

            if i.key_pressed(egui::Key::Enter) {
                if let Some(idx) = self.selected_index {
                    if let Some(issue) = self.issues.get(idx) {
                        self.current_view = View::Detail(issue.id.clone());
                    }
                }
            }
        });
    }

    fn sortable_header_ui(&self, ui: &mut egui::Ui, label: &str, column: SortColumn) -> bool {
        let text = if self.sort_by == column {
            format!("{} {}", label, if self.sort_ascending { "▲" } else { "▼" })
        } else {
            label.to_string()
        };

        ui.button(text).clicked()
    }

    fn show_detail_view(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame, issue_id: &str) {
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
        let mut nav_to_issue = None;

        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("← Back").clicked() {
                    self.current_view = View::List;
                    self.current_issue = None;
                    self.edit_modified = false;
                    self.refresh();
                }
                ui.separator();
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
                }
            });

            if let Some(ref error) = self.error_message {
                ui.colored_label(egui::Color32::RED, error);
            }

            if self.edit_modified {
                ui.colored_label(egui::Color32::YELLOW, "Unsaved changes");
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
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
                    let desc_edit = egui::TextEdit::multiline(&mut issue.description)
                        .desired_width(f32::INFINITY);
                    if ui.add(desc_edit).changed() {
                        self.edit_modified = true;
                    }

                    if !issue.dependencies.is_empty() {
                        ui.separator();
                        ui.label("Dependencies (Blocks this issue):");
                        for dep in &issue.dependencies {
                            ui.horizontal(|ui| {
                                if ui.button(&dep.id).clicked() {
                                    nav_to_issue = Some(dep.id.clone());
                                }
                                ui.label(format!("- {}", dep.title));
                            });
                        }
                    }
                }
            });
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

        if let Some(new_issue_id) = nav_to_issue {
            self.current_view = View::Detail(new_issue_id);
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

        // Update description
        if let Err(e) = BdClient::update_issue(&issue.id, "description", &issue.description) {
            errors.push(format!("description: {}", e));
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
        match &self.current_view {
            View::List => self.show_list_view(ctx, frame),
            View::Detail(issue_id) => {
                let id = issue_id.clone();
                self.show_detail_view(ctx, frame, &id);
            }
        }
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
