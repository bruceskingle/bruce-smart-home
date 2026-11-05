# myenergi

The myenergi module fetches data about myenergi devices, such as the zappi EV charger, from myenergi's servers. Each zappi's records of what it charged the car with, and of its grid CT clamp, are channels which the [Site](../site.md) page can place, so that the [Electricity](../electricity.md) page splits the import with them. Its own pages show what it fetches:

- **Days**, the totals of each device's records of each day, in kWh. **Fetch** gets the status of every device and the last few days of their records. Days are UTC days, and a day is fetched again until it has ended.
- **Cache Files**, every file it has fetched, as myenergi sent it.

The zappi's charge added is only kept until the car is next plugged in, so while the app is closed or the computer is asleep a charging session's total can be missed. A [capture device](../devices.md) with its **Myenergi zappi** feature enabled polls the status every 5 minutes instead, whether the app is running or not, and the app reads what it took when it next polls, and saves it as if it had polled myenergi itself. The app still fetches each device's minute records from myenergi, as myenergi keeps those.

[Logging in](login.md) explains what it needs to fetch your data.
