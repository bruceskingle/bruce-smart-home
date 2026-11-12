//! A button showing a date range, which opens a dialog to change it.
//! Times are selected in half hour slots in a given timezone.

use bruce_graphql::types::{Date, DateTime};
use dioxus::prelude::*;
use dioxus_free_icons::{
    Icon,
    icons::ld_icons::{LdCalendar, LdChevronLeft, LdChevronRight},
};
use time_tz::{OffsetDateTimeExt, PrimitiveDateTimeExt, Tz};

/// The number of half hour slots in a day.
const SLOTS_PER_DAY: u8 = 48;

/// Format a half hour slot as HH:MM
fn slot_label(slot: u8) -> String {
    format!("{:02}:{:02}", slot / 2, (slot % 2) * 30)
}

/// A wall clock date and half hour time slot in the billing timezone.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LocalSlot {
    pub date: time::Date,
    /// Half hours since midnight, 0..SLOTS_PER_DAY
    pub slot: u8,
}

impl LocalSlot {
    fn time(&self) -> time::Time {
        time::Time::from_hms(self.slot / 2, (self.slot % 2) * 30, 0).unwrap_or(time::Time::MIDNIGHT)
    }

    /// The slot containing the time, as a unix timestamp, in the timezone.
    pub fn from_unix_timestamp(timestamp: i64, tz: &Tz) -> Option<Self> {
        let local = time::OffsetDateTime::from_unix_timestamp(timestamp)
            .ok()?
            .to_timezone(tz);
        Some(LocalSlot {
            date: local.date(),
            slot: local.hour() * 2 + local.minute() / 30,
        })
    }

    /// None if this wall clock time does not exist in the timezone, i.e. when the clocks go forward.
    /// When the clocks go back the earlier of the two possible times is used.
    pub fn to_date_time(&self, tz: &Tz) -> Option<DateTime> {
        let offset_date_time = time::PrimitiveDateTime::new(self.date, self.time())
            .assume_timezone(tz)
            .take_first()?;
        DateTime::from_unix_timestamp(offset_date_time.unix_timestamp())
            .ok()
            .map(|date_time| date_time.to_timezone(tz))
    }
}

impl std::fmt::Display for LocalSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.date, slot_label(self.slot))
    }
}

/// The selected period, an end of None means up to the present.
#[derive(Clone, Copy, PartialEq)]
pub struct RangeSelection {
    pub start: LocalSlot,
    pub end: Option<LocalSlot>,
}

impl RangeSelection {
    /// From the start of the current month in the given timezone, up to now.
    pub fn current_month(tz: &Tz) -> Self {
        let today = crate::clock::today(tz);
        RangeSelection {
            start: LocalSlot {
                date: today.replace_day(1).unwrap_or(today),
                slot: 0,
            },
            end: None,
        }
    }

    /// From start to the exclusive end, as unix timestamps, in the timezone.
    pub fn from_unix_timestamps(start: i64, end: i64, tz: &Tz) -> Option<Self> {
        Some(RangeSelection {
            start: LocalSlot::from_unix_timestamp(start, tz)?,
            end: Some(LocalSlot::from_unix_timestamp(end, tz)?),
        })
    }

    /// The whole day containing the time, as a unix timestamp, in the timezone.
    pub fn day_containing(timestamp: i64, tz: &Tz) -> Option<Self> {
        let date = LocalSlot::from_unix_timestamp(timestamp, tz)?.date;
        Some(RangeSelection {
            start: LocalSlot { date, slot: 0 },
            end: Some(LocalSlot {
                date: date.next_day()?,
                slot: 0,
            }),
        })
    }

    /// The start and exclusive end of the range, up to now is returned as an end of midnight at the start of tomorrow.
    pub fn to_date_times(&self, tz: &'static Tz) -> Result<(DateTime, DateTime), String> {
        let not_exist = |slot: &LocalSlot| format!("{slot} does not exist because the clocks go forward");

        let start = self.start.to_date_time(tz).ok_or_else(|| not_exist(&self.start))?;
        let end = match &self.end {
            Some(end_slot) => end_slot.to_date_time(tz).ok_or_else(|| not_exist(end_slot))?,
            None => crate::clock::today_date(tz).next_day().at_midnight(tz),
        };
        if end <= start {
            return Err(String::from("The to time must be after the from time"));
        }
        Ok((start, end))
    }
}

/// The length of a range, for paging to the one before or after it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RangeLength {
    /// Calendar months, for ranges from the 1st of a month.
    Months(i32),
    /// Whole days, so that paging across a clock change keeps midnight as midnight.
    Days(i64),
    /// Anything else.
    Seconds(i64),
}

/// The date the given number of months after date, which is the 1st of a month.
fn add_months(date: time::Date, months: i32) -> Option<time::Date> {
    let index = date.year() * 12 + date.month() as i32 - 1 + months;
    time::Date::from_calendar_date(
        index.div_euclid(12),
        time::Month::try_from((index.rem_euclid(12) + 1) as u8).ok()?,
        1,
    )
    .ok()
}

/// The number of months from the month of start to the month of end.
fn months_between(start: time::Date, end: time::Date) -> i32 {
    (end.year() - start.year()) * 12 + end.month() as i32 - start.month() as i32
}

impl RangeSelection {
    /// The length of the range when today is the given date and now the given time. A range up to now counts as the
    /// whole of today, and as a week when it starts on Monday this week (This Week), so that the range before it is the
    /// same period as the Last and Prev buttons give.
    fn length(&self, today: time::Date, now: &DateTime, tz: &'static Tz) -> Option<RangeLength> {
        let start = self.start;
        Some(match self.end {
            Some(end) if start.slot == 0 && end.slot == 0 => {
                if start.date.day() == 1 && end.date.day() == 1 {
                    RangeLength::Months(months_between(start.date, end.date))
                } else {
                    RangeLength::Days((end.date - start.date).whole_days())
                }
            }
            Some(end) => {
                RangeLength::Seconds(end.to_date_time(tz)?.unix_timestamp() - start.to_date_time(tz)?.unix_timestamp())
            }
            None if start.slot == 0 => {
                let days = (today - start.date).whole_days();
                if start.date.day() == 1 {
                    RangeLength::Months(months_between(start.date, today) + 1)
                } else if start.date.weekday() == time::Weekday::Monday && days < 7 {
                    RangeLength::Days(7)
                } else {
                    RangeLength::Days(days.max(1))
                }
            }
            None => RangeLength::Seconds(now.unix_timestamp() - start.to_date_time(tz)?.unix_timestamp()),
        })
    }

    /// The range of the same length immediately before this one and after it, now, None if there is no range after it
    /// because it reaches now.
    pub fn before_and_after(&self, tz: &'static Tz) -> (Option<RangeSelection>, Option<RangeSelection>) {
        let now = crate::clock::now(tz);
        let today = crate::clock::today(tz);
        (self.previous(today, &now, tz), self.next(today, &now, tz))
    }

    /// The range of the same length immediately before this one.
    fn previous(&self, today: time::Date, now: &DateTime, tz: &'static Tz) -> Option<RangeSelection> {
        let start = self.start;
        let new_start = match self.length(today, now, tz)? {
            RangeLength::Months(months) => LocalSlot {
                date: add_months(start.date, -months)?,
                slot: 0,
            },
            RangeLength::Days(days) => LocalSlot {
                date: start.date - time::Duration::days(days),
                slot: start.slot,
            },
            RangeLength::Seconds(seconds) => {
                LocalSlot::from_unix_timestamp(start.to_date_time(tz)?.unix_timestamp() - seconds, tz)?
            }
        };
        Some(RangeSelection {
            start: new_start,
            end: Some(start),
        })
    }

    /// The range of the same length immediately after this one, which is up to now if it would reach now. None if this
    /// range already reaches now.
    fn next(&self, today: time::Date, now: &DateTime, tz: &'static Tz) -> Option<RangeSelection> {
        let end = self.end?;
        if end.to_date_time(tz)? >= *now {
            return None;
        }
        let new_end = match self.length(today, now, tz)? {
            RangeLength::Months(months) => LocalSlot {
                date: add_months(end.date, months)?,
                slot: 0,
            },
            RangeLength::Days(days) => LocalSlot {
                date: end.date + time::Duration::days(days),
                slot: end.slot,
            },
            RangeLength::Seconds(seconds) => {
                LocalSlot::from_unix_timestamp(end.to_date_time(tz)?.unix_timestamp() + seconds, tz)?
            }
        };
        let reaches_now = new_end.to_date_time(tz).is_none_or(|new_end| new_end >= *now);
        Some(RangeSelection {
            start: end,
            end: if reaches_now { None } else { Some(new_end) },
        })
    }
}

/// A button in the date range dialog which sets a range from 00:00 on a day, either up to now or, for the previous
/// periods, to 00:00 when the current one starts.
#[derive(Clone, Copy, PartialEq)]
enum QuickRange {
    /// From the most recent Monday.
    ThisWeek,
    /// From the same day last week, so a little over 7 days.
    SevenDays,
    /// From the 1st of this month.
    ThisMonth,
    /// From 30 days before today, so a little over 30 days.
    ThirtyDays,
    /// The week before this week, Monday to Monday.
    LastWeek,
    /// The 7 days before SevenDays.
    PrevSevenDays,
    /// The calendar month before this one.
    LastMonth,
    /// The 30 days before ThirtyDays.
    PrevThirtyDays,
}

impl QuickRange {
    /// The buttons in two rows, the current periods above the previous ones.
    const ROWS: [[QuickRange; 4]; 2] = [
        [
            QuickRange::ThisWeek,
            QuickRange::SevenDays,
            QuickRange::ThisMonth,
            QuickRange::ThirtyDays,
        ],
        [
            QuickRange::LastWeek,
            QuickRange::PrevSevenDays,
            QuickRange::LastMonth,
            QuickRange::PrevThirtyDays,
        ],
    ];

    fn label(&self) -> &'static str {
        match self {
            QuickRange::ThisWeek => "This Week",
            QuickRange::SevenDays => "7 Days",
            QuickRange::ThisMonth => "This Month",
            QuickRange::ThirtyDays => "30 Days",
            QuickRange::LastWeek => "Last Week",
            QuickRange::PrevSevenDays => "Prev 7 Days",
            QuickRange::LastMonth => "Last Month",
            QuickRange::PrevThirtyDays => "Prev 30 Days",
        }
    }

    /// The range when today is the given date.
    fn selection(&self, today: time::Date) -> RangeSelection {
        let days_ago = |days: i64| today - time::Duration::days(days);
        let this_week = days_ago(today.weekday().number_days_from_monday() as i64);
        let this_month = today.replace_day(1).unwrap_or(today);
        let last_month = this_month
            .previous_day()
            .map_or(this_month, |day| day.replace_day(1).unwrap_or(day));

        let (start, end) = match self {
            QuickRange::ThisWeek => (this_week, None),
            QuickRange::SevenDays => (days_ago(7), None),
            QuickRange::ThisMonth => (this_month, None),
            QuickRange::ThirtyDays => (days_ago(30), None),
            QuickRange::LastWeek => (this_week - time::Duration::days(7), Some(this_week)),
            QuickRange::PrevSevenDays => (days_ago(14), Some(days_ago(7))),
            QuickRange::LastMonth => (last_month, Some(this_month)),
            QuickRange::PrevThirtyDays => (days_ago(60), Some(days_ago(30))),
        };
        let midnight = |date| LocalSlot { date, slot: 0 };
        RangeSelection {
            start: midnight(start),
            end: end.map(midnight),
        }
    }
}

impl std::fmt::Display for RangeSelection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.end {
            Some(end) => write!(f, "{} to {}", self.start, end),
            None => write!(f, "{} to now", self.start),
        }
    }
}

/// The values in the date range dialog, which may be incomplete while being edited.
#[derive(Clone)]
struct RangeDraft {
    start_date: Option<time::Date>,
    start_slot: u8,
    end_date: Option<time::Date>,
    end_slot: u8,
    to_now: bool,
}

impl RangeDraft {
    fn new(selection: &RangeSelection) -> Self {
        // When there is no end, offer the start date as a starting point for the user to edit.
        let end = selection.end.unwrap_or(selection.start);
        Self {
            start_date: Some(selection.start.date),
            start_slot: selection.start.slot,
            end_date: Some(end.date),
            end_slot: end.slot,
            to_now: selection.end.is_none(),
        }
    }

    fn to_selection(&self, tz: &'static Tz) -> Result<RangeSelection, String> {
        let start_date = self.start_date.ok_or_else(|| String::from("Select a from date"))?;
        let end = if self.to_now {
            None
        } else {
            let end_date = self
                .end_date
                .ok_or_else(|| String::from("Select a to date, or tick Up to now"))?;
            Some(LocalSlot {
                date: end_date,
                slot: self.end_slot,
            })
        };
        let selection = RangeSelection {
            start: LocalSlot {
                date: start_date,
                slot: self.start_slot,
            },
            end,
        };

        // Check that the times exist and are in the right order.
        selection.to_date_times(tz)?;
        Ok(selection)
    }
}

/// An empty or unparsable value clears the date.
fn parse_date_input(value: String) -> Option<time::Date> {
    value.parse::<Date>().ok().map(|date| *date)
}

/// Select element offering each half hour of the day.
fn slot_select(selected: u8, disabled: bool, onchange: impl FnMut(FormEvent) + 'static) -> Element {
    rsx! {
        select { disabled, onchange,
            for slot in 0..SLOTS_PER_DAY {
                option { value: "{slot}", selected: slot == selected, {slot_label(slot)} }
            }
        }
    }
}

#[derive(Props, Clone)]
pub struct DateRangePickerProps {
    /// The applied range, which is updated when the user applies a change.
    range: Signal<RangeSelection>,
    timezone: &'static Tz,
}

impl PartialEq for DateRangePickerProps {
    fn eq(&self, other: &Self) -> bool {
        self.range == other.range && std::ptr::eq(self.timezone, other.timezone)
    }
}

#[component]
pub fn DateRangePicker(props: DateRangePickerProps) -> Element {
    let tz = props.timezone;
    let mut range = props.range;

    // The contents of the date range dialog, Some while the dialog is open.
    let mut draft = use_signal::<Option<RangeDraft>>(|| None);

    let selection = *range.read();

    let dialog = if let Some(current) = draft.read().clone() {
        let validated = current.to_selection(tz);
        let draft_error = validated.as_ref().err().cloned();
        let can_apply = validated.is_ok();
        let start_value = current.start_date.map(|d| d.to_string()).unwrap_or_default();
        let end_value = current.end_date.map(|d| d.to_string()).unwrap_or_default();

        // Apply a change to the draft.
        let mut edit = move |change: &dyn Fn(&mut RangeDraft)| {
            if let Some(d) = draft.write().as_mut() {
                change(d);
            }
        };

        rsx! {
            div { class: "modal-backdrop",
                form {
                    class: "modal",
                    onsubmit: move |event: FormEvent| {
                        event.prevent_default();
                        if let Ok(selection) = &validated {
                            range.set(*selection);
                            draft.set(None);
                        }
                    },
                    h3 { "Select period" }
                    div { class: "modal-row",
                        label { "From" }
                        input {
                            r#type: "date",
                            value: "{start_value}",
                            onchange: move |event| edit(&|d| d.start_date = parse_date_input(event.value())),
                        }
                        {
                            slot_select(
                                current.start_slot,
                                false,
                                move |event| {
                                    if let Ok(slot) = event.value().parse::<u8>() {
                                        edit(&|d| d.start_slot = slot)
                                    }
                                },
                            )
                        }
                    }
                    div { class: "modal-row",
                        label { "To" }
                        input {
                            r#type: "date",
                            value: "{end_value}",
                            disabled: current.to_now,
                            onchange: move |event| edit(&|d| d.end_date = parse_date_input(event.value())),
                        }
                        {
                            slot_select(
                                current.end_slot,
                                current.to_now,
                                move |event| {
                                    if let Ok(slot) = event.value().parse::<u8>() {
                                        edit(&|d| d.end_slot = slot)
                                    }
                                },
                            )
                        }
                    }
                    div { class: "modal-row",
                        label {}
                        label {
                            input {
                                r#type: "checkbox",
                                checked: current.to_now,
                                onchange: move |event| edit(&|d| d.to_now = event.checked()),
                            }
                            " Up to now"
                        }
                    }
                    if let Some(error) = draft_error {
                        div { class: "modal-error", "{error}" }
                    }
                    div { class: "modal-footer",
                        div { class: "quick-ranges",
                            for quick in QuickRange::ROWS.into_iter().flatten() {
                                button {
                                    r#type: "button",
                                    onclick: move |_| {
                                        let today = crate::clock::today(tz);
                                        range.set(quick.selection(today));
                                        draft.set(None);
                                    },
                                    {quick.label()}
                                }
                            }
                        }
                        div { class: "modal-buttons",
                            button {
                                r#type: "button",
                                onclick: move |_| draft.set(None),
                                "Cancel"
                            }
                            button { r#type: "submit", disabled: !can_apply, "Apply" }
                        }
                    }
                }
            }
        }
    } else {
        rsx! {}
    };

    // The same period before and after the selected one, there is nothing after a period which reaches now.
    let (previous, next) = selection.before_and_after(tz);

    rsx! {
        span { class: "range-pager",
            button {
                r#type: "button",
                title: "The same period before",
                disabled: previous.is_none(),
                onclick: move |_| {
                    if let Some(previous) = previous {
                        range.set(previous);
                    }
                },
                Icon {
                    class: "inline-icon",
                    icon: LdChevronLeft,
                    width: 14,
                    height: 14,
                }
            }
            button {
                r#type: "button",
                title: "Change the period",
                onclick: move |_| draft.set(Some(RangeDraft::new(&range.read()))),
                Icon {
                    class: "inline-icon",
                    icon: LdCalendar,
                    width: 14,
                    height: 14,
                }
                " {selection}"
            }
            button {
                r#type: "button",
                title: "The same period after",
                disabled: next.is_none(),
                onclick: move |_| {
                    if let Some(next) = next {
                        range.set(next);
                    }
                },
                Icon {
                    class: "inline-icon",
                    icon: LdChevronRight,
                    width: 14,
                    height: 14,
                }
            }
        }
        {dialog}
    }
}

#[cfg(test)]
mod tests {
    use time::macros::{date, datetime};

    use bruce_graphql::types::DateTime;

    use super::{LocalSlot, QuickRange, RangeSelection};

    #[test]
    fn ranges_are_made_from_timestamps_in_the_local_time_of_the_timezone() {
        let london = time_tz::timezones::db::europe::LONDON;
        // Midnight BST on 2026-09-01 to midnight BST on 2026-09-08, a week bar.
        let start = datetime!(2026-08-31 23:00 UTC).unix_timestamp();
        let end = datetime!(2026-09-07 23:00 UTC).unix_timestamp();

        let range = RangeSelection::from_unix_timestamps(start, end, london).unwrap();
        assert!(
            range.start
                == LocalSlot {
                    date: date!(2026 - 09 - 01),
                    slot: 0
                }
        );
        assert!(
            range.end
                == Some(LocalSlot {
                    date: date!(2026 - 09 - 08),
                    slot: 0
                })
        );

        // 2026-09-01 15:30 BST is in the 31st half hour slot of the day.
        let afternoon = datetime!(2026-09-01 14:30 UTC).unix_timestamp();
        assert!(
            LocalSlot::from_unix_timestamp(afternoon, london)
                == Some(LocalSlot {
                    date: date!(2026 - 09 - 01),
                    slot: 31
                })
        );
        let day = RangeSelection::day_containing(afternoon, london).unwrap();
        assert!(
            day.start
                == LocalSlot {
                    date: date!(2026 - 09 - 01),
                    slot: 0
                }
        );
        assert!(
            day.end
                == Some(LocalSlot {
                    date: date!(2026 - 09 - 02),
                    slot: 0
                })
        );
    }

    #[test]
    fn quick_ranges_start_at_midnight_and_run_up_to_now() {
        // A Wednesday.
        let today = date!(2026 - 10 - 21);
        let start = |quick: QuickRange| {
            let selection = quick.selection(today);
            assert!(selection.end.is_none());
            assert_eq!(selection.start.slot, 0);
            selection.start.date
        };
        assert_eq!(start(QuickRange::ThisWeek), date!(2026 - 10 - 19));
        assert_eq!(start(QuickRange::SevenDays), date!(2026 - 10 - 14));
        assert_eq!(start(QuickRange::ThisMonth), date!(2026 - 10 - 01));
        assert_eq!(start(QuickRange::ThirtyDays), date!(2026 - 09 - 21));

        // On a Monday this week starts today.
        assert_eq!(
            QuickRange::ThisWeek.selection(date!(2026 - 10 - 19)).start.date,
            date!(2026 - 10 - 19)
        );
    }

    #[test]
    fn previous_quick_ranges_end_where_the_current_ones_start() {
        let today = date!(2026 - 10 - 21);
        let range = |quick: QuickRange| {
            let selection = quick.selection(today);
            (selection.start.date, selection.end.map(|end| end.date))
        };
        assert_eq!(
            range(QuickRange::LastWeek),
            (date!(2026 - 10 - 12), Some(date!(2026 - 10 - 19)))
        );
        assert_eq!(
            range(QuickRange::PrevSevenDays),
            (date!(2026 - 10 - 07), Some(date!(2026 - 10 - 14)))
        );
        assert_eq!(
            range(QuickRange::LastMonth),
            (date!(2026 - 09 - 01), Some(date!(2026 - 10 - 01)))
        );
        assert_eq!(
            range(QuickRange::PrevThirtyDays),
            (date!(2026 - 08 - 22), Some(date!(2026 - 09 - 21)))
        );

        // Last month from January is December of the year before.
        assert_eq!(
            QuickRange::LastMonth.selection(date!(2027 - 01 - 15)).start.date,
            date!(2026 - 12 - 01)
        );
    }

    #[test]
    fn paging_moves_by_the_same_period_and_stops_at_now() {
        let london = time_tz::timezones::db::europe::LONDON;
        // Wednesday 21 October 2026, 15:00 BST.
        let today = date!(2026 - 10 - 21);
        let now = DateTime::from_unix_timestamp(datetime!(2026-10-21 14:00 UTC).unix_timestamp()).unwrap();
        let slot = |date| LocalSlot { date, slot: 0 };
        let range = |start, end: Option<time::Date>| RangeSelection {
            start: slot(start),
            end: end.map(slot),
        };
        let previous = |selection: RangeSelection| selection.previous(today, &now, london).unwrap();
        let next = |selection: RangeSelection| selection.next(today, &now, london);

        // Before the up to now ranges are the same as the Last and Prev buttons.
        for quick in [
            QuickRange::ThisWeek,
            QuickRange::SevenDays,
            QuickRange::ThisMonth,
            QuickRange::ThirtyDays,
        ] {
            assert!(next(quick.selection(today)).is_none());
        }
        assert!(previous(QuickRange::ThisWeek.selection(today)) == QuickRange::LastWeek.selection(today));
        assert!(previous(QuickRange::SevenDays.selection(today)) == QuickRange::PrevSevenDays.selection(today));
        assert!(previous(QuickRange::ThisMonth.selection(today)) == QuickRange::LastMonth.selection(today));
        assert!(previous(QuickRange::ThirtyDays.selection(today)) == QuickRange::PrevThirtyDays.selection(today));

        // Months page by calendar month, including across a year.
        assert!(
            previous(range(date!(2026 - 01 - 01), Some(date!(2026 - 02 - 01))))
                == range(date!(2025 - 12 - 01), Some(date!(2026 - 01 - 01)))
        );
        assert!(
            next(range(date!(2026 - 01 - 01), Some(date!(2026 - 02 - 01))))
                == Some(range(date!(2026 - 02 - 01), Some(date!(2026 - 03 - 01))))
        );

        // Days page by whole days, across the clock change on 25 October.
        assert!(
            next(range(date!(2026 - 10 - 01), Some(date!(2026 - 10 - 03))))
                == Some(range(date!(2026 - 10 - 03), Some(date!(2026 - 10 - 05))))
        );
        assert!(
            previous(range(date!(2026 - 10 - 26), Some(date!(2026 - 10 - 27))))
                == range(date!(2026 - 10 - 25), Some(date!(2026 - 10 - 26)))
        );

        // After last week is this week, up to now.
        assert!(next(QuickRange::LastWeek.selection(today)) == Some(QuickRange::ThisWeek.selection(today)));
    }
}
