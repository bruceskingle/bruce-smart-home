# Settings

These are the settings of the active domain, each domain has its own. ![Settings](icons/settings.svg) at the top right opens them.

## Log

[The log](log.md) is kept in a file in the data directory, and this chooses when it is started again:

- **Keep at least the last N hours**, the default being 24. When the log's first entry is older than that, the log is renamed with *.old* added to its name, replacing the previous one, and a new log is started. So between them the two files always have at least the last N hours.
- **Start the log again each time the app is opened**, so that the log has just this run of the app, and the *.old* file the run before.
- **Keep up to N MB in all**, starting the log again when it reaches half that size.

Entries are never deleted from the log, it is only ever renamed, so another copy of the app writing to it at the same time, such as a [scheduled fetch](../ScheduledFetch.md), never loses any.

Click **Save** to keep the changes.
