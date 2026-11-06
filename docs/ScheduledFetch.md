# Fetching Data on a Schedule

Marco Sparko can fetch the latest data without opening its window, and then exit. Run it like this on a schedule, e.g.
once a week, and the app is always up to date when you open it.

It matters most for data which Octopus only keeps for a short time. The Home Mini's readings every 10 seconds are only
available for 30 days, so if the app is not opened for longer than that, the readings in between are lost for good. A
scheduled fetch saves them as it goes.

## What it Does

```
marco-sparko --fetch
```

fetches what every page would fetch if you opened it with the date range set to the last 30 days: bills (and the
details of those issued in the last 30 days), properties, each meter's readings, each charger's charging sessions and
the Home Mini's readings. First it copies what the profile's capture devices have recorded since it last did, from
those whose firmware keeps a store; those with older firmware are read when the modules are. Anything already downloaded and up to date is not downloaded again, so a run is quick when
little has changed. It prints what it is doing, and exits with status 0 if everything was fetched, or 1 if anything
failed, e.g. a network problem or a login which has expired, in which case open the app and log in again.

It uses the profile the app last used. To use another one, add `--profile <name>`, and if the data has been moved from
your home directory (see [Profiles](gettingStarted/profiles.md#moving-the-data-directory)), add `--home <directory>` or set the same
environment variable for the scheduled task as for the app.

You must have opened the app and logged in at least once, as the scheduled fetch uses the saved login.

## How Often

Once a week is plenty. It must be at least every 30 days for the Home Mini's readings, and more often leaves some margin
for the computer being off or asleep. Octopus limits the Home Mini's readings to 125 requests an hour, and a week of
them takes about 14, so any frequency is fine. If the limit is reached, the fetch waits until it can continue.

The schedulers below run a missed fetch when the computer next wakes up or starts, so a fetch due while it was off is
not lost.

## macOS

On macOS, use launchd rather than cron: it runs a fetch which was missed while the Mac was asleep as soon as it wakes,
and needs no extra permissions.

1. Create the file `~/Library/LaunchAgents/org.skingle.marco-sparko.fetch.plist` with this, which fetches every Sunday
   at 03:00 (`Weekday` 0 is Sunday, 1 is Monday, and so on), replacing `YOU` with your user name:

    ```xml
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
        <key>Label</key>
        <string>org.skingle.marco-sparko.fetch</string>
        <key>ProgramArguments</key>
        <array>
            <string>/Applications/MarcoSparko.app/Contents/MacOS/marco-sparko</string>
            <string>--fetch</string>
        </array>
        <key>StartCalendarInterval</key>
        <dict>
            <key>Weekday</key>
            <integer>0</integer>
            <key>Hour</key>
            <integer>3</integer>
            <key>Minute</key>
            <integer>0</integer>
        </dict>
        <key>StandardOutPath</key>
        <string>/Users/YOU/Library/Logs/MarcoSparko/fetch.log</string>
        <key>StandardErrorPath</key>
        <string>/Users/YOU/Library/Logs/MarcoSparko/fetch.log</string>
    </dict>
    </plist>
    ```

2. In Terminal, create the log directory and load the schedule:

    ```
    mkdir -p ~/Library/Logs/MarcoSparko
    launchctl bootstrap gui/$(id -u) ~/Library/LaunchAgents/org.skingle.marco-sparko.fetch.plist
    ```

3. Try it now rather than waiting until Sunday, and look at the log, which can also be seen in the Console app:

    ```
    launchctl kickstart gui/$(id -u)/org.skingle.marco-sparko.fetch
    cat ~/Library/Logs/MarcoSparko/fetch.log
    ```

To fetch every day instead, at 03:00, leave out the `Weekday` key and its value, so that `StartCalendarInterval` is:

```xml
<key>StartCalendarInterval</key>
<dict>
    <key>Hour</key>
    <integer>3</integer>
    <key>Minute</key>
    <integer>0</integer>
</dict>
```

To change the schedule, edit the file, then unload it and load it again. To stop the scheduled fetches altogether, unload
it and delete the file:

```
launchctl bootout gui/$(id -u)/org.skingle.marco-sparko.fetch
rm ~/Library/LaunchAgents/org.skingle.marco-sparko.fetch.plist
```

It runs while you are logged in, which includes a Mac which is asleep or has its screen locked.

## Windows

On Windows, use Task Scheduler. In PowerShell (not as administrator), this creates a task which fetches every Sunday at
03:00, or as soon as possible after that if the computer was off, and writes what it did to `MarcoSparko-fetch.log` in
your local app data folder:

```powershell
$exe = "$env:LOCALAPPDATA\Programs\MarcoSparko\marco-sparko.exe"
$log = "$env:LOCALAPPDATA\MarcoSparko-fetch.log"
$action = New-ScheduledTaskAction -Execute "cmd.exe" -Argument "/c `"`"$exe`" --fetch > `"$log`" 2>&1`""
$trigger = New-ScheduledTaskTrigger -Weekly -DaysOfWeek Sunday -At 3am
$settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName "Marco Sparko fetch" -Action $action -Trigger $trigger -Settings $settings
```

To fetch every day instead, at 03:00, use this trigger in place of the weekly one when creating the task:

```powershell
$trigger = New-ScheduledTaskTrigger -Daily -At 3am
```

or change the trigger of a task which already exists:

```powershell
Set-ScheduledTask -TaskName "Marco Sparko fetch" -Trigger (New-ScheduledTaskTrigger -Daily -At 3am)
```

Try it now, then look at the log:

```powershell
Start-ScheduledTask -TaskName "Marco Sparko fetch"
Get-Content "$env:LOCALAPPDATA\MarcoSparko-fetch.log"
```

The task can also be seen, changed and run from the Task Scheduler app, under Task Scheduler Library. It runs while you
are logged in, and a command window may appear briefly while it does. To remove it:

```powershell
Unregister-ScheduledTask -TaskName "Marco Sparko fetch"
```

If Marco Sparko was installed somewhere else, change `$exe` to where `marco-sparko.exe` is.

## Linux

A crontab entry works, e.g. with `crontab -e`, every Sunday at 03:00:

```
0 3 * * 0 /path/to/marco-sparko --fetch >> ~/.marco-sparko-fetch.log 2>&1
```

or every day at 03:00:

```
0 3 * * * /path/to/marco-sparko --fetch >> ~/.marco-sparko-fetch.log 2>&1
```

cron does not run a fetch which was missed while the computer was off. A systemd user timer with `Persistent=true`
does, if that matters.
