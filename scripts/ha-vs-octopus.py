#!/usr/bin/env python3
"""Compares Home Assistant's history of an EV charger with what Octopus billed, from the app's old Octopus cache, in the layout before 0.5.0.

Three questions:
- Was each half hour billed at the low or the standard rate, and does Home Assistant's off peak sensor agree?
- When charging runs on past the end of a half hour, what was the next half hour billed at?
- How do Octopus's charging session totals compare with the charger's own energy added since plug in, and with the
  charger's power readings added up over time? Each session is compared until the next one starts, as Octopus's end
  of a session is when the car was to be ready by, and charging often carries on after it.

Export the history from Home Assistant's History page (Download data) with the charger's charge added and charge power
sensors and the Octopus integration's off peak binary sensor, then run, e.g.

    scripts/ha-vs-octopus.py --profile default history.csv [more.csv ...]

Several exports can be given, e.g. for consecutive date ranges. Only half hours and sessions inside the history are
compared. Half hours Octopus has not made final may still change.
"""

import argparse
import collections
import csv
import datetime
import glob
import json
import os

UTC = datetime.timezone.utc
HALF_HOUR = datetime.timedelta(minutes=30)


def parse_time(text):
    return datetime.datetime.fromisoformat(text.replace("Z", "+00:00")).astimezone(UTC)


def number(text):
    try:
        return float(text)
    except ValueError:
        return None


def read_history(paths):
    """The states of each entity, in time order, from Home Assistant's History CSVs."""
    series = collections.defaultdict(set)
    for path in paths:
        with open(path) as file:
            for row in csv.DictReader(file):
                series[row["entity_id"]].add((parse_time(row["last_changed"]), row["state"]))
    return {entity: sorted(states) for entity, states in series.items()}


def entity(series, *fragments):
    """The states of the entity whose id contains all of fragments."""
    matches = [name for name in series if all(fragment in name for fragment in fragments)]
    if len(matches) != 1:
        raise SystemExit(f"Expected one entity matching {fragments}, found {matches or 'none'}")
    return series[matches[0]]


def read_json_lines(pattern):
    for path in sorted(glob.glob(pattern)):
        with open(path) as file:
            for line in file:
                if line.strip():
                    yield json.loads(line)


def read_half_hours(cache):
    """Octopus's half hours of the import meter, the one with EV rates, by start time: units by rate, and whether
    final."""
    half_hours = {}
    for record in read_json_lines(os.path.join(cache, "*", "*#MeasurementsConsumption")):
        units = {name: float(value["units"]) for name, value in record["consumption"].items()}
        if any(name.startswith("EV") for name in units):
            half_hours[parse_time(record["start"])] = (units, record["is_final"])
    return half_hours


def units(half_hour, *prefixes):
    return sum(value for name, value in half_hour.items() if name.startswith(prefixes))


def value_at(states, time):
    """The last numeric state at or before time."""
    found = None
    for at, state in states:
        if at > time:
            break
        found = number(state) if number(state) is not None else found
    return found


def on_fraction(states, start):
    """The fraction of the half hour from start that the binary sensor was on."""
    end = start + HALF_HOUR
    state = None
    for at, value in states:
        if at <= start:
            state = value
    on = 0.0
    current = start
    for at, value in states:
        if start < at < end:
            if state == "on":
                on += (at - current).total_seconds()
            current, state = at, value
    if state == "on":
        on += (end - current).total_seconds()
    return on / HALF_HOUR.total_seconds()


def integrated_kwh(power, start, end):
    """The charger's power readings, in kW, added up from start to end, each held until the next."""
    total = 0.0
    previous = None
    for at, state in power:
        kw = number(state)
        if previous is not None:
            a, b = max(previous[0], start), min(at, end)
            if b > a:
                total += previous[1] * (b - a).total_seconds() / 3600
        previous = (at, kw or 0.0)
    return total


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--profile", required=True, help="the profile whose old Octopus cache to read")
    parser.add_argument("--home", default=os.path.expanduser("~"), help="the data home, holding .bruce-smart-home-data")
    parser.add_argument("history", nargs="+", help="Home Assistant History CSVs")
    args = parser.parse_args()

    cache = os.path.join(args.home, ".bruce-smart-home-data", "old-cache", f"{args.profile}-octopus")
    series = read_history(args.history)
    off_peak = entity(series, "off_peak")
    charge_added = entity(series, "charge_added")
    power = entity(series, "charge_power")
    start = max(off_peak[0][0], charge_added[0][0])
    end = min(off_peak[-1][0], charge_added[-1][0])
    half_hours = read_half_hours(cache)
    print(f"History from {start:%Y-%m-%d %H:%M} to {end:%Y-%m-%d %H:%M} UTC")

    print("\n== Half hours where Home Assistant's off peak sensor was not simply right ==")
    agree = differ = empty = 0
    for at in sorted(half_hours):
        if not (start <= at and at + HALF_HOUR <= end):
            continue
        half_hour, final = half_hours[at]
        low, standard = units(half_hour, "EV Low", "Home Low"), units(half_hour, "EV Standard", "Home Standard")
        if low + standard < 0.005:
            empty += 1
            continue
        billed = "low" if standard < 0.0005 else "peak" if low < 0.0005 else "mixed"
        fraction = on_fraction(off_peak, at)
        if fraction in (0.0, 1.0) and billed == ("low" if fraction else "peak"):
            agree += 1
            continue
        differ += 1
        verdict = "agrees" if billed == ("low" if fraction > 0.5 else "peak") else "WRONG"
        print(f"  {at:%d %H:%M}  billed {billed:5} (low {low:.3f}, standard {standard:.3f} kWh)  "
              f"off peak for {fraction:4.0%}  {verdict}{'' if final else '  (not final)'}")
    print(f"  {agree} half hours agree, {differ} above, {empty} with no consumption")

    print("\n== Charging which ran on into the next half hour ==")
    runs = []
    charging = None
    for at, state in power:
        kw = number(state)
        if kw is None:
            continue
        if kw > 0.5 and charging is None:
            charging = at
        elif kw < 0.5 and charging is not None:
            runs.append((charging, at))
            charging = None
    for run_start, run_end in runs:
        boundary = run_end.replace(minute=0 if run_end.minute < 30 else 30, second=0, microsecond=0)
        if not (start <= run_start and run_end <= end) or boundary <= run_start or run_end - boundary > datetime.timedelta(minutes=5):
            continue
        after = (value_at(charge_added, run_end) or 0) - (value_at(charge_added, boundary) or 0)
        half_hour, final = half_hours.get(boundary, ({}, None))
        state = "not cached" if final is None else "" if final else "  (not final)"
        print(f"  ended {run_end:%d %H:%M:%S}, {after:.2f} kWh after {boundary:%H:%M}: Octopus billed EV low "
              f"{units(half_hour, 'EV Low'):.3f}, EV standard {units(half_hour, 'EV Standard'):.3f} kWh {state}")

    print("\n== Charging sessions: Octopus's energy added, the charger's, and its power added up ==")
    # Only charging at home, not e.g. public rapid charging which Octopus lists too, compared until the next session
    # starts, as a session's end is the time the car was to be ready by, and charging often carries on after it.
    sessions = sorted((session for session in read_json_lines(os.path.join(cache, "*", "*#ChargingSessions"))
        if session.get("__typename") == "SmartFlexChargingSession"), key=lambda session: session["start"])
    for index, session in enumerate(sessions):
        session_start = parse_time(session["start"])
        window_end = parse_time(sessions[index + 1]["start"]) if index + 1 < len(sessions) else end
        octopus = float(session["energyAdded"]["value"])
        if not (start <= session_start and window_end <= end):
            continue
        added = [number(state) for at, state in charge_added if session_start <= at < window_end and number(state) is not None]
        highest = max(added) if added else float("nan")
        integrated = integrated_kwh(power, session_start, window_end)
        if octopus == 0 and integrated < 0.01:
            continue
        print(f"  {session_start:%d %H:%M} to {window_end:%d %H:%M}  Octopus {octopus:6.2f}  charger {highest:6.2f}  "
              f"power added up {integrated:6.2f} kWh")


if __name__ == "__main__":
    main()
