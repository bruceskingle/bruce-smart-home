# Domains

A domain is a separate set of logins, downloaded data and settings, e.g. one for your own account and one for a
relative's. The domain menu at the top right, ![Domain](icons/domain.svg) and the domain's name, changes between them,
and **Create New Domain** adds one. The **…** beside each domain in the menu has:

- **Size...**, this page.
- **Copy...**, which copies the domain, its settings and its downloaded data, but not its log, as a new domain, e.g. to
  try a change, such as its [topology](site.md), without changing the original. An **Offline** copy, as it is to start
  with, shows the data as it is when it is copied, without logging in or fetching anything, and has neither the
  original's logins nor its [capture devices](devices.md). A copy which is not offline has both, and fetches and polls
  as the original does when it is the active domain.
- **Rename...**, which renames the domain, its downloaded data and its log. The active domain can not be renamed, as the
  app has its files open; change to another one first.
- **Delete...**, see below.

This page shows how much disk space a domain's data takes up, for each module. A domain which is not the active one can
be deleted here, which cannot be undone. The domain, its settings and its logins are deleted at once. Its downloaded
data and its log are moved to the bin (the Recycle Bin on Windows), so the disk space they use is only freed when the
bin is emptied. The active domain cannot be deleted, change to another one first.

See [Domains](../gettingStarted/domains.md) in the documentation for more, including how to move the data directory.
