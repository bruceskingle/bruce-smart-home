# Installing on macOS

How to download Bruce's Smart Home for your Mac and install it in Applications. The app is signed with a Developer ID
and notarized by Apple, so it opens like any other app downloaded from the internet.

## Download the Release Binary

Go to [https://github.com/bruceskingle/bruce-smart-home/releases/](https://github.com/bruceskingle/bruce-smart-home/releases/)
to find the latest release:

![Download Page](../download.png)

Look for the one with the <img src="../latest.png" width="60"> label, and download the `.dmg` for your Mac:
`BrucesSmartHome_0.5.0_aarch64.dmg` for a Mac with Apple silicon, e.g. an M1 or later, or
`BrucesSmartHome_0.5.0_x86_64.dmg` for an Intel Mac. The version number, `0.5.0` here, may be different. **About This
Mac**, in the Apple menu, says which chip your Mac has.

## Install It

When the file has downloaded, click the **Downloads** icon in the Dock and click the `.dmg` file to open it, or open it
from the **Downloads** folder in **Finder**.

<img src="Screenshot1.png" width="275">

It opens as a drive, like the image below. Drag the app's icon onto the **Applications** shortcut.

<img src="Screenshot2.png" width="550">

If you have installed the app before, macOS asks whether to replace it. Click **Replace**.

<img src="Screenshot3.png" width="550">

Then right click in the drive's window and choose **Eject "BrucesSmartHome"**.

<img src="Screenshot4.png" width="550">

The first time you open the app, macOS says that it was downloaded from the internet and asks whether to open it.
Click **Open**.

Versions before 0.5.0 were not signed, and needed the command `xattr -d com.apple.quarantine` to open. That is no
longer needed.

## The Keychain

After you have saved a login, the Keychain may ask for your Mac's password, so that the app can read the key which
unlocks your logins, see [Your secrets and the Keychain](../cachedData.md#your-secrets-and-the-keychain).

## From the Command Line

To run the app from the command line or a script, e.g. a [scheduled fetch](../../ScheduledFetch.md), its full path is

```
/Applications/BrucesSmartHome.app/Contents/MacOS/bruce-smart-home
```
