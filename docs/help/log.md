# The Log

The log records the important things the app does, such as each request made to a supplier, how much data it returned and how long it took, and anything which went wrong. Errors are shown in red and warnings in yellow.

It is shown along the bottom of the window, and ![Log](icons/log.svg) at the top right shows or hides it, as does its ![Close](icons/close.svg). Drag its top edge to make it taller or shorter.

It includes what a [scheduled fetch](../ScheduledFetch.md) does, even while the app is open, marked **fetch** in the column beside the time. Times are in your computer's time zone.

The log is kept in a file for each domain, in the data directory, and the [settings](settings.md) choose how much of it is kept.
