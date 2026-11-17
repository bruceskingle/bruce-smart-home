# Temperature and Humidity

Some capture devices have a temperature and humidity sensor of their own, such as the Waveshare ESP32-S3-RLCD-4.2.
Unlike the data they fetch from the Powerwall, the zappi or Octopus, these are their own readings, which no one else
has, so each device's are kept separately, and a house with several devices has a line for each room.

To start, open the device's **Settings** on the Devices page, enable **Temperature and Humidity Log**, click **Save**,
then **Restart Now**. The device reads its sensor every 10 seconds, and the app reads the readings from it every 5
minutes, while it is open, see Polling. The device keeps them on its SD card until then, or without one, the last two
days or so in its memory.

As the temperature and humidity change slowly, the device keeps them as periods, from one time to another at one value,
which last while the readings stay within 0.1 °C, or 1 % of humidity, of it, rather than a reading every 10 seconds.
The period still going on is read again each time, with its end moved on.

- The table at the top shows each device's latest reading, and when it was.
- Choose the period to show with the dates, see [Date ranges](date-range.md): a day, a week, a month, or any other.
- The graphs show the temperature, in °C, and the humidity, in %, a line for each device, in the colours of the key
  above them. A line which stops means the device was not measuring, e.g. it was restarting.

A sensor on a board is warmed by it, so it reads high. Correct it with the **Temperature Offset** setting of the device's
**Temperature and Humidity** feature, in tenths of a degree, found by comparing it with a thermometer which can be
trusted, placed next to it for half an hour. The humidity is corrected with it.
