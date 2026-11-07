# Capture and Display Devices for bruce-smart-home

Firmware for small, always-on ESP32 boards with screens, which poll a household's Tesla Powerwalls, myenergi zappi and
Octopus Energy smart charging while the computer running the app sleeps, keep what they fetch on an SD card until the
app reads it, and show the latest on their screens.

Each release attaches the firmware for each board it runs on: an ESP32-2432S028R "Cheap Yellow Display", a Waveshare
ESP32-C6-Touch-LCD-1.47, a Waveshare ESP32-S3-RLCD-4.2 and a Guition JC3248W535C. The app's Devices page flashes it
to a board plugged in by USB, and adopts the board once it is on the Wi-Fi.
