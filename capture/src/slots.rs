//! The slots of a poll's schedule, so that each record says which it is for, as the app's polling does, see
//! bruce-smart-home's src/polling.rs.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use croner::Cron;

pub struct Slots {
    schedule: Cron,
    next: DateTime<Utc>,
}

impl Slots {
    /// spec is the schedule the poll's task was added with.
    pub fn new(spec: &str) -> anyhow::Result<Slots> {
        let schedule = Cron::from_str(spec)?;
        let next = schedule.find_next_occurrence(&Utc::now(), false)?;
        Ok(Slots { schedule, next })
    }

    /// The slot a run is for, the latest time of the schedule which has passed, as the scheduler runs a task at or a
    /// little after its time, later if another task held it up.
    pub fn current(&mut self) -> anyhow::Result<Option<DateTime<Utc>>> {
        let now = Utc::now();
        let mut slot = None;
        while self.next <= now {
            slot = Some(self.next);
            self.next = self.schedule.find_next_occurrence(&self.next, false)?;
        }
        Ok(slot)
    }
}
