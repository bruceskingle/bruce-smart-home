# Running Bruce's Smart Home

The first time you run Bruce's Smart Home you will see a window like this:


<img src="Screenshot1.png" width="1024">

This is the ```Home``` screen which lists all the available Modules (there is ony Octopus at the moment) as well as displaying the timestamp when the release was created and the name of the Git branch it was built from. ```(detached)``` just means that this executable was built from a release tag in the source code repository which is normal for a formal release. You can ignore this information. Notice that the Octopus module shows as ```[inactive]``` because we have not logged in yet.

Along the right of the window is the help, which describes the screen you are looking at, with links to more detailed
help. Click ![Help](../help/icons/help.svg) at the top right to hide it, or show it again.

Clickable links in the user interface will highlight if you hover the mouse over them. In this case the ```Home``` item on the menu bar at the top (which brings you back to this page) and the names of modules in the list (just ```octopus``` at the moment) are clickable.

On macOS, a window from the Keychain may ask for your Mac's password, so that Bruce's Smart Home can read the key which
unlocks the logins it keeps, e.g. your Octopus API key. You will see it after you have saved a login, the first time a
new version of the app starts, e.g. after an update. Enter your password and choose **Always Allow**; see
[Your secrets and the Keychain](cachedData.md#your-secrets-and-the-keychain) for why it asks.

The first step is to login to your Octopus account.
