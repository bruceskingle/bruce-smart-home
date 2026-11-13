# Energy accounting

Notes for coding agents changing Home's pages or `src/power/`, moved from AGENTS.md.

- **The site's topology is never assumed.** Where each meter and CT clamp measures differs from house to house, so the
  split of the import by use comes from the profile's topology, `src/power/topology.rs`, set on Home's Site page, see
  [docs/PowerSchema.md](../docs/PowerSchema.md). Say "topology", not wiring, and that a circuit feeds another, or is fed
  from it, not "under" it, as the diagram goes from left to right.
- **What the supplier says, and what happened.** A module's pages show what its source says, e.g. Octopus's Meters
  page, including its split of the car's import, which Octopus models, e.g. the same energy in each half hour of a
  charge. Home's pages are the app's best understanding from everything available, e.g. the measured split of the
  import into the house's, the car's and the batteries', so they never present a supplier's split by use as fact. But
  the supplier's bill is a fact: in every bar on Home's pages the kWh and the cost at each rate, e.g. peak and
  off-peak, agree with the supplier's, and only who used them is the app's. A supplier's rate may depend on what it
  took the energy to be for, e.g. Octopus billed the car's charging at peak, inside the cheap times, past a limit with
  the charge cap off, while the house's was cheap; so the kWh it billed as a use go to that use as far as it was
  measured to use them, and only the rest to other uses, at the rate they were billed at. This is done through the
  shared schema, `src/power/`, from the rates, uses and costs a module gives with its readings, not by naming a
  supplier. The Home Mini is a consumer access
  device reading the same meter Octopus bills from.
