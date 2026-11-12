# Installing on macOS

How to download Bruce's Smart Home for your Mac, install it in Applications, and allow it to run. The app is not signed by
Apple, so there is one extra step after installing it, which removes the warning macOS would otherwise show.

## Download the Release Binary

Navigate to [https://github.com/bruceskingle/bruce-smart-home/releases/](https://github.com/bruceskingle/bruce-smart-home/releases/) to find the
latest release: 

![Download Page](../download.png)

Look for the one with the <img src="../latest.png" width="60"> label, and then select the correct version for your computer, either
```BrucesSmartHome_0.4.2_x64.dmg``` for Intel based computers or ```BrucesSmartHome_0.4.2_aarch64.dmg``` for Apple Silicone based computers (the version number ```0.4.2``` may be different of course).

When the file has downloaded click on the ```Downloads``` icon in the Dock and click on the ```.dmg``` file to open it. Alternatively open ```Finder``` go to the ```Downloads``` folder and click on it from there.

<img src="Screenshot1.png" width="275">

The distribution file will open as an attached drive like the image below, drag and drop the application icon onto the Applications shortcut.

<img src="Screenshot2.png" width="550">

If you have installed the application before you will see this dialog, if so then click ```Replace```

<img src="Screenshot3.png" width="550">

Finally right click on the distribution file window and select on ```Eject "BrucesSmartHome"```

<img src="Screenshot4.png" width="550">

Since this app is not signed you will need to open a ```Terminal``` window and execute the following command to remove the file from quarantine:

```
xattr -d com.apple.quarantine /Applications/BrucesSmartHome.app 
```

If you wish to execute the app from the command line directly or from a shell script, the full path to the executable is
```
/Applications/BrucesSmartHome.app/Contents/MacOS/bruce-smart-home
```
