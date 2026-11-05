# Site

Home's Site page says where in your house each module measures, its **topology**, from the grid connection through the circuits each one feeds, so that the [Electricity](electricity.md) page can split the electricity you import from the grid into the car's, the batteries' and the house's. It differs from house to house: a Tesla gateway's grid clamp is usually fitted where the grid comes in, so it measures the car's charger too, but some houses have it moved so that the car is outside it. The app can not tell which yours is, so you say. Until you do, the import is shown by the rate it was billed at only.

## The diagram

The diagram draws the grid on the left, and what each circuit feeds to its right, in the colour of what it is for: grey for the grid, white for a board or gateway, blue for the car, purple for a battery, yellow for solar panels and green for the house. Under each circuit's name are its **channels**, what measures it, e.g. `tesla:site`, the gateway's site meter. A circuit with no channel says **the rest**: it is what feeds it less what its other circuits measured, e.g. a car beside the gateway has the grid's import less the gateway's.

## Editing it

Until it is set, the page checks the last 7 days' measurements straight away, and shows a first topology which they fit, if they say enough for one, see [Checks](#checks). Otherwise, or while they are being checked, it shows one made from what your modules' channels usually measure: the grid, measured by your supplier's meter and a zappi's grid clamp, the Tesla gateway fed from it, feeding the Powerwalls, the solar panels, the car, as installers usually fit it, and the house. **Start from this** lets you change it. Where your car's charger is only you know: if it is outside the gateway, remove the car from the gateway and add a circuit for the car fed from the grid, with **Add fed circuit** on the grid, with the zappi's channel, or with no channel if there is no zappi.

Where a circuit and the circuits it feeds are all measured, e.g. the grid feeding a car measured by its zappi and a gateway, which rarely add up exactly, its import is shared between them in proportion to what each measured.

Each row is a circuit, indented below the circuit which feeds it:

- Its **name**, shown in the diagram.
- What it is **for**: the grid (only the top circuit), a board or gateway which feeds others, a battery, solar panels, or a load of one of the categories, see below, e.g. the car or the house.
- **Measured by**: click a channel to put it on the circuit or take it off. Its tooltip says what it is. The channels are listed under the diagram too.
- **Add fed circuit** adds a circuit which it feeds, and **Remove** removes it, with every circuit it feeds.

Only one of a circuit's circuits can be without a channel, as how the rest is shared between two can not be told, and a channel can only measure one circuit. The page says what is wrong in red, and **Save** waits until it is right. **Undo changes** goes back to what was saved, and **Clear** forgets the topology, so the import is shown by rate only again.

## Categories

A load's category is what the Electricity page splits the import into: the car's is **EV**, e.g. EV Low Rate, the house's **Home**, and any other's its own name, e.g. **Hot water Low Rate**. **Car**, **House** and **Unknown**, for the rest of what a board feeds when what it is is not known, are built in. **Add category** adds another, e.g. Hot water or Heat pump, which every circuit can then be, and **Remove** removes one no circuit is. Each category's colour is its colour in the diagram, and the default of its labels in the graphs, whose Categories menus can change each label's.

## Checks

**Check** loads what every channel measured over the last 7 days, half hour by half hour, and checks the topology against it. It checks the topology as it is being edited, before it is saved, so you can try placing a channel in different circuits and see which fits. Each channel's figure is its **net import**: what went into its circuit less what came out, e.g. a battery's charge less its discharge, or minus what the solar panels made. Energy is conserved, so:

- Where a circuit and every circuit it feeds have channels, what they feed adds up to it: e.g. **Grid feeds Zappi, Gateway** compares the grid's with the zappi's and the gateway's together.
- Where a circuit has two channels, they agree: e.g. **Gateway is measured by tesla:site and myenergi:…:grid**.
- A circuit with no channel, the rest of what feeds it, is never less than none: if it is, something it is said to be beside is really fed from it, e.g. the car behind the gateway's clamp.

Above the checks it says whether the topology **looks right**: every check within 10%, and the rest less than none in no more than 2% of its half hours. If it does not, it shows a topology made from what the measurements say, see below, which does fit them, if they say enough for one. **Use this** puts it in the editor, to change, e.g. the circuits' names, and **Save**. Only you know the house, so look at it first: the measurements can not tell e.g. two loads which are always on together apart.

**Out by** is how far apart the two sides are, each half hour's difference, whichever way, added up, as a share of the total: green up to 5%, red over 15%. A few percent is usual, as the meters are read at slightly different moments and are not all as accurate. **Furthest out** shows the half hours furthest apart, to look at on the Electricity page.

### What the measurements say

Under the checks, whatever the topology, the measurements say which channels measure the same circuit, which measures another's the other way round, as a clamp put on backwards would, and which channel's net import is the sum of two, three or four others', as a circuit feeding them would be, each to within 5%. E.g. a zappi's grid clamp moved beside a Tesla gateway's says it measures the same as `tesla:site`, and the grid's meter is the zappi's and the gateway's. Use them to place the channels.

## Your channels

- **Octopus**, `octopus:electricity`: the meter Octopus bills from, its half hourly readings, and, if you have one, the [Home Mini's](octopus/home-mini.md), which reads the same meter every 10 seconds.
- **GloBird**, `globird:electricity`: the meter GloBird bills from, every 5 minutes.
- **myenergi**, for each zappi: `myenergi:Z` and its serial number, what it charged the car with, and the same with `:grid`, its grid CT clamp, if one of its clamps is set to Grid. They are from the zappi's records of each minute, which myenergi keeps. myenergi's records miss a minute here and there, about 70 a day; a gap of one or two minutes is filled in with the average of the minutes either side, and a half hour with a longer gap is not split.
- **Tesla**, read by a [capture device](devices.md): `tesla:site`, the gateway's site meter, its grid clamp; `tesla:battery`, the Powerwalls; `tesla:solar`, the solar panels; and `tesla:load`, the house, as the gateway works it out from the others.
