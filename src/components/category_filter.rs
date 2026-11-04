//! The categories of measurement shown in graphs, e.g. the standing charge or the consumption at one rate, with the
//! colour, line style and visibility chosen for each, and the colour chosen for each year in the trend graph.

use std::collections::BTreeMap;

use dioxus::prelude::*;
use dioxus_free_icons::{Icon, icons::ld_icons::{LdChevronDown, LdFilter}};

use crate::components::color_panel::move_color_panel_to_mouse;

pub const STANDING_CHARGE_LABEL: &str = "Standing charge";
pub const TOTAL_LABEL: &str = "Total";

const STANDING_CHARGE_COLOR: &str = "#b8c0cc";
const TOTAL_COLOR: &str = "#f5f5f5";

/// The labels of the current home and EV rates with their default colours, in the order they are shown, then those of
/// the Electricity page, see electricity.rs. Readings may have other labels, e.g. older readings have time of use
/// buckets, which come after these.
pub const KNOWN_LABELS: [(&str, &str); 15] = [
    ("Home Low Rate", "#46ee43"),
    ("Home Standard Rate", "#f6671f"),
    ("EV Low Rate", "#271cff"),
    ("EV Standard Rate", "#f72525"),
    // The Powerwalls charging from the grid, where it is measured, see electricity::measured_split, in purple as in the
    // user's Home Assistant.
    ("Battery Low Rate", "#9b51e0"),
    ("Battery Standard Rate", "#d946ef"),
    (super::electricity::LOW_ESTIMATED_LABEL, "#a6f5a4"),
    (super::electricity::STANDARD_ESTIMATED_LABEL, "#fab48f"),
    ("EV Low Rate (estimated)", "#8f8aff"),
    ("EV Standard Rate (estimated)", "#fa8f8f"),
    ("Battery Low Rate (estimated)", "#c9a7f2"),
    ("Battery Standard Rate (estimated)", "#f0abfc"),
    (super::electricity::UNMETERED_LABEL, "#8a8a99"),
    (super::electricity::EXPORT_LABEL, "#f59e0b"),
    (super::electricity::OCTOPUS_EV_LABEL, "#ffffff"),
];

/// Default colours for labels which are not in KNOWN_LABELS, used in turn.
const OTHER_COLORS: [&str; 6] = ["#f2c94c", "#bb6bd9", "#2dd4bf", "#f472b6", "#a3e635", "#60a5fa"];

/// The line styles of categories in the trend graph, as a name and an SVG stroke-dasharray, used in turn by default.
pub const LINE_STYLES: [(&str, &str); 6] = [
    ("Solid", "none"),
    ("Dashed", "8 4"),
    ("Dotted", "2 3"),
    ("Dash dot", "12 4 2 4"),
    ("Short dash", "4 4"),
    ("Sparse dots", "1 5"),
];

/// The line style of a line in the trend graph when there is only one category.
pub const SOLID: &str = "none";

/// Default colours of the years in the trend graph, used in turn.
const YEAR_COLORS: [&str; 6] = ["#06b6d4", "#f6671f", "#46ee43", "#bb6bd9", "#f2c94c", "#f472b6"];

/// The colour of the line style samples, which show only the style.
const SAMPLE_COLOR: &str = "#c4c4ce";

#[derive(Clone, Debug, PartialEq)]
pub struct CategoryStyle {
    /// A CSS colour, e.g. #46ee43
    pub color: String,
    pub visible: bool,
}

/// The colours, line styles and visibility chosen for each category, and the colours chosen for each year, anything
/// which has not been changed has its default. There is also a total, which when visible replaces the visible categories
/// with their total. The trend graph has its own total, which is visible by default.
#[derive(Clone, Debug, PartialEq)]
pub struct CategoryFilter {
    styles: BTreeMap<String, CategoryStyle>,
    dashes: BTreeMap<String, String>,
    year_colors: BTreeMap<i32, String>,
    total: CategoryStyle,
    trend_total: CategoryStyle,
}

impl Default for CategoryFilter {
    fn default() -> Self {
        Self {
            styles: BTreeMap::new(),
            dashes: BTreeMap::new(),
            year_colors: BTreeMap::new(),
            total: CategoryStyle { color: String::from(TOTAL_COLOR), visible: false },
            trend_total: CategoryStyle { color: String::from(TOTAL_COLOR), visible: true },
        }
    }
}

impl CategoryFilter {
    /// The style of each of the given categories, in the same order.
    pub fn styles(&self, categories: &[String]) -> Vec<CategoryStyle> {
        let mut other_colors = OTHER_COLORS.iter().cycle();
        categories.iter()
            .map(|label| {
                // Work out the default for every category so that the palette colours do not depend on which have
                // been changed.
                let default_color = if label == STANDING_CHARGE_LABEL {
                    STANDING_CHARGE_COLOR
                }
                else {
                    KNOWN_LABELS.iter()
                        .find(|(known, _)| known == label)
                        .map(|(_, color)| *color)
                        .unwrap_or_else(|| other_colors.next().unwrap())
                };
                self.styles.get(label)
                    .cloned()
                    .unwrap_or_else(|| CategoryStyle { color: String::from(default_color), visible: true })
            })
            .collect()
    }

    pub fn set_style(&mut self, label: &str, style: CategoryStyle) {
        self.styles.insert(label.to_string(), style);
    }

    /// The line style of each of the given categories in the trend graph, as an SVG stroke-dasharray.
    pub fn dashes(&self, categories: &[String]) -> Vec<String> {
        categories.iter()
            .enumerate()
            .map(|(index, label)| self.dashes.get(label)
                .cloned()
                .unwrap_or_else(|| String::from(LINE_STYLES[index % LINE_STYLES.len()].1)))
            .collect()
    }

    pub fn set_dash(&mut self, label: &str, dash: String) {
        self.dashes.insert(label.to_string(), dash);
    }

    /// The colour of each of the given years in the trend graph.
    pub fn year_colors(&self, years: &[i32]) -> Vec<String> {
        years.iter()
            .enumerate()
            .map(|(index, year)| self.year_colors.get(year)
                .cloned()
                .unwrap_or_else(|| String::from(YEAR_COLORS[index % YEAR_COLORS.len()])))
            .collect()
    }

    pub fn set_year_color(&mut self, year: i32, color: String) {
        self.year_colors.insert(year, color);
    }

    /// The total for the trend graph if trend, otherwise for the bar graph.
    pub fn total(&self, trend: bool) -> &CategoryStyle {
        if trend { &self.trend_total } else { &self.total }
    }

    pub fn set_total(&mut self, trend: bool, style: CategoryStyle) {
        if trend {
            self.trend_total = style;
        }
        else {
            self.total = style;
        }
    }
}

/// A category to draw, with the value for each point of the graph.
pub struct CategoryValues {
    pub label: String,
    /// The colour in the bar graph.
    pub color: String,
    /// The line style in the trend graph.
    pub dash: String,
    pub values: Vec<f64>,
}

impl CategoryFilter {
    /// The visible categories, or if the total is visible a single category with their total.
    /// categories gives the label and the value for each point of every category, trend chooses the total of the trend graph.
    pub fn apply(&self, categories: Vec<(String, Vec<f64>)>, points: usize, trend: bool) -> Vec<CategoryValues> {
        let labels: Vec<String> = categories.iter().map(|(label, _)| label.clone()).collect();
        let visible: Vec<CategoryValues> = categories.into_iter()
            .zip(self.styles(&labels))
            .zip(self.dashes(&labels))
            .filter(|((_, style), _)| style.visible)
            .map(|(((label, values), style), dash)| CategoryValues { label, color: style.color, dash, values })
            .collect();

        let total = self.total(trend);
        if total.visible {
            let values = (0..points)
                .map(|index| visible.iter().map(|category| category.values.get(index).copied().unwrap_or_default()).sum())
                .collect();
            vec![CategoryValues { label: String::from(TOTAL_LABEL), color: total.color.clone(), dash: String::from(SOLID), values }]
        }
        else {
            visible
        }
    }
}

/// A button with a menu of the categories, each with a colour picker and a check box to show or hide it, and a total.
/// For the trend graph each category has a line style picker in place of the colour picker, and there is a colour
/// picker for each of the years.
#[component]
/// With visibility_only, the categories only have a check box, and there is no total, e.g. for categories which filter
/// what is shown rather than being drawn themselves.
pub fn CategoryFilterMenu(filter: Signal<CategoryFilter>, categories: Vec<String>, years: Vec<i32>, trend: bool, disabled: bool,
    #[props(default)] visibility_only: bool) -> Element {
    let mut menu_open = use_signal(|| false);
    let styles = filter.read().styles(&categories);
    let dashes = filter.read().dashes(&categories);
    let year_colors = filter.read().year_colors(&years);
    let total = filter.read().total(trend).clone();

    rsx! {
        span { class: "menu-anchor",
            button {
                r#type: "button",
                title: match (visibility_only, trend) {
                    (true, true) => "Choose the categories shown and the colours of the years",
                    (true, false) => "Choose the categories shown",
                    (false, true) => "Choose the categories shown, their line styles and the colours of the years",
                    (false, false) => "Choose the categories shown and their colours",
                },
                disabled,
                onclick: move |_| menu_open.set(!menu_open()),
                Icon {
                    class: "inline-icon",
                    icon: LdFilter,
                    width: 14,
                    height: 14,
                }
                " Categories "
                Icon {
                    class: "inline-icon",
                    icon: LdChevronDown,
                    width: 14,
                    height: 14,
                }
            }
            if menu_open() && !disabled {
                // Covers the page so that clicking anywhere else closes the menu.
                div { class: "menu-backdrop", onclick: move |_| menu_open.set(false) }
                div { class: "dropdown-menu category-menu",
                    for ((label, style), dash) in categories.iter().cloned().zip(styles).zip(dashes) {
                        div { class: "category-row",
                            if visibility_only {}
                            else if trend {
                                {line_style_picker(filter, label.clone(), dash)}
                            }
                            else {
                                {color_picker(&label, style.color.clone(), {
                                    let label = label.clone();
                                    let style = style.clone();
                                    move |color| {
                                        let mut filter = filter;
                                        filter.write().set_style(&label, CategoryStyle { color, ..style.clone() });
                                    }
                                })}
                            }
                            {visibility_check_box(filter, label.clone(), style.clone(), None)}
                        }
                    }
                    if !visibility_only {
                        div { class: "menu-separator" }
                        div { class: "category-row",
                            // In the trend graph the total has the colour of each year, and a solid line.
                            if !trend {
                                {color_picker(TOTAL_LABEL, total.color.clone(), {
                                    let total = total.clone();
                                    move |color| {
                                        let mut filter = filter;
                                        filter.write().set_total(trend, CategoryStyle { color, ..total.clone() });
                                    }
                                })}
                            }
                            {visibility_check_box(filter, String::from(TOTAL_LABEL), total.clone(), Some(trend))}
                        }
                    }
                    if trend {
                        div { class: "menu-separator" }
                        for (year, color) in years.iter().copied().zip(year_colors) {
                            div { class: "category-row",
                                {color_picker(&year.to_string(), color, move |color| {
                                    let mut filter = filter;
                                    filter.write().set_year_color(year, color);
                                })}
                                span { "{year}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A colour picker, which calls set with the colour chosen.
fn color_picker(name: &str, color: String, set: impl Fn(String) + 'static) -> Element {
    rsx! {
        input {
            r#type: "color",
            title: "Choose the colour of {name}",
            value: "{color}",
            // On macOS this makes "Show Colors…" open next to the picker rather than wherever it was last left.
            onclick: move |_| move_color_panel_to_mouse(),
            oninput: move |event: FormEvent| set(event.value()),
        }
    }
}

/// A list of the line styles, with a sample of the one chosen.
fn line_style_picker(filter: Signal<CategoryFilter>, label: String, dash: String) -> Element {
    rsx! {
        svg { class: "line-style-sample", width: 28, height: 12,
            line { x1: 2, x2: 26, y1: 6, y2: 6, stroke: SAMPLE_COLOR, stroke_width: 2, stroke_dasharray: "{dash}" }
        }
        select {
            title: "Choose the line style of {label}",
            onchange: {
                let label = label.clone();
                move |event: FormEvent| {
                    let mut filter = filter;
                    filter.write().set_dash(&label, event.value());
                }
            },
            for (name, style) in LINE_STYLES {
                option { value: style, selected: style == dash, "{name}" }
            }
        }
    }
}

/// A check box which shows or hides the category, or the total if total_for_trend is given, with the label.
fn visibility_check_box(filter: Signal<CategoryFilter>, label: String, style: CategoryStyle, total_for_trend: Option<bool>) -> Element {
    rsx! {
        label {
            input {
                r#type: "checkbox",
                checked: style.visible,
                onchange: {
                    let label = label.clone();
                    move |event: FormEvent| {
                        let mut filter = filter;
                        let style = CategoryStyle { visible: event.checked(), ..style.clone() };
                        if let Some(trend) = total_for_trend {
                            filter.write().set_total(trend, style);
                        }
                        else {
                            filter.write().set_style(&label, style);
                        }
                    }
                },
            }
            " {label}"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CategoryFilter, CategoryStyle, STANDING_CHARGE_LABEL, TOTAL_LABEL};

    fn labels(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|label| label.to_string()).collect()
    }

    #[test]
    fn known_labels_keep_their_colours_and_others_use_the_palette() {
        let styles = CategoryFilter::default().styles(&labels(&[STANDING_CHARGE_LABEL, "Home Low Rate", "OFF_PEAK", "STANDARD_RATE"]));
        let colors: Vec<&str> = styles.iter().map(|style| style.color.as_str()).collect();
        assert_eq!(colors, ["#b8c0cc", "#46ee43", "#f2c94c", "#bb6bd9"]);
        assert!(styles.iter().all(|style| style.visible));
    }

    #[test]
    fn changing_one_category_does_not_change_the_default_colours_of_the_others() {
        let mut filter = CategoryFilter::default();
        filter.set_style("OFF_PEAK", CategoryStyle { color: String::from("#000000"), visible: true });
        let colors: Vec<String> = filter.styles(&labels(&["OFF_PEAK", "STANDARD_RATE"])).into_iter().map(|style| style.color).collect();
        assert_eq!(colors, ["#000000", "#bb6bd9"]);
    }

    #[test]
    fn hidden_categories_are_left_out_and_the_total_replaces_the_visible_ones() {
        let categories = || vec![
            (String::from("A"), vec![1.0, 2.0]),
            (String::from("B"), vec![10.0, 20.0]),
            (String::from("C"), vec![100.0, 200.0]),
        ];
        let mut filter = CategoryFilter::default();
        filter.set_style("B", CategoryStyle { color: String::from("#000000"), visible: false });

        let shown: Vec<String> = filter.apply(categories(), 2, false).into_iter().map(|category| category.label).collect();
        assert_eq!(shown, ["A", "C"]);

        filter.set_total(false, CategoryStyle { color: String::from("#ffffff"), visible: true });
        let total = filter.apply(categories(), 2, false);
        assert_eq!(total.len(), 1);
        assert_eq!(total[0].label, TOTAL_LABEL);
        assert_eq!(total[0].values, [101.0, 202.0]);
    }

    #[test]
    fn the_total_is_shown_by_default_only_in_the_trend_graph() {
        let filter = CategoryFilter::default();
        assert!(!filter.total(false).visible);
        assert!(filter.total(true).visible);
    }

    #[test]
    fn line_styles_and_year_colours_have_defaults_in_turn_which_can_be_changed() {
        let mut filter = CategoryFilter::default();
        assert_eq!(filter.dashes(&labels(&["A", "B"])), ["none", "8 4"]);
        filter.set_dash("A", String::from("2 3"));
        assert_eq!(filter.dashes(&labels(&["A", "B"])), ["2 3", "8 4"]);

        assert_eq!(filter.year_colors(&[2025, 2026]), ["#06b6d4", "#f6671f"]);
        filter.set_year_color(2026, String::from("#000000"));
        assert_eq!(filter.year_colors(&[2025, 2026]), ["#06b6d4", "#000000"]);
    }
}
