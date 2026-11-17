"""Shows the log of an ESP32 plugged in by USB, without restarting it, until Ctrl+C.

espflash monitor --no-reset still connects to the chip's bootloader, which stops the program, so this only reads the
serial port. The board resets the chip when DTR and RTS differ, and macOS sets both when the port is opened, so they are
both kept set: turning them off, one at a time, restarts it.

Usage: python3 log.py [port], by default the first /dev/cu.usbserial-*. Needs pyserial.
"""

import glob
import sys

import serial


def main():
    ports = sys.argv[1:] or sorted(glob.glob("/dev/cu.usbserial-*"))
    if not ports:
        sys.exit("No /dev/cu.usbserial-* port, is the device plugged in?")
    port = serial.Serial()
    port.port = ports[0]
    port.baudrate = 115200
    port.dtr = True
    port.rts = True
    # So that espflash fails at once while this runs, rather than waiting to connect, as this holds the reset lines.
    port.exclusive = True
    port.open()
    print(f"Showing the log from {port.port}, Ctrl+C to stop", flush=True)
    try:
        while True:
            line = port.readline()
            if line:
                sys.stdout.write(line.decode("utf-8", errors="replace"))
                sys.stdout.flush()
    except KeyboardInterrupt:
        pass
    finally:
        port.close()


if __name__ == "__main__":
    main()
