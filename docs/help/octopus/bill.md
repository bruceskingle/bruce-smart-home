# A Bill

One bill, in its own window, so that the bill list stays where it was, and bills can be compared side by side. Close it with its close button or Escape. In a bill's window ![Close](../icons/close.svg) hides the help, and ![Help](../icons/help.svg) at the top right shows it again. The **Window** menu in the main window lists the open bills, and can arrange them across the screen.

## Summary of Charges

Each transaction on the bill, such as a charge for a period's electricity, a payment or a credit. Where a charge is for energy, it shows the units and the cost per unit, both with and without the standing charge. The totals are at the bottom.

## Detailed Breakdown

For each charge, the tariff it was billed on and the cost of each rate, checked against the statement: the totals worked out from the tariff should match those on the bill. The **Consumption Analysis** shows how much of the cost and units was at each rate.

On a half-hourly tariff, such as Intelligent Octopus Go, the tariff shows its **Unit Rates**: the price of each type of rate, e.g. EV_DEVICE_OFF_PEAK for charging the car at the cheap rate, and when it applies from. Octopus only gives a tariff as it is now, e.g. the rates which apply now, which may not be those of an older bill. So that changes can be seen, the tariff in force is kept, whole, with when it was first seen, in the `TariffHistory` file in the profile's cache, and again whenever anything in it changes, such as its rates or standing charge. The log says when one changes. Once a tariff has ended it is left as it was last kept, as Octopus no longer gives its rates.

## Open Bills

A bill which has not been issued yet is **open**. Octopus does not show it in its app, its charges, dates and reference may change until it is issued, and it may be replaced by a different bill.
